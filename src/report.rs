use crate::ai::{AiClient, ProviderConfig};
use crate::cli::Cli;
use crate::config;
use crate::file_reader::{
    build_file_selection, format_selected_files_for_prompt, read_selected_files,
};
use crate::inventory::{build_inventory, Inventory};
use anyhow::{bail, Context};
use colored::Colorize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuditVerdict {
    Safe,
    Suspicious,
    Unsafe,
}

impl AuditVerdict {
    fn is_blocking(self) -> bool {
        matches!(self, AuditVerdict::Suspicious | AuditVerdict::Unsafe)
    }

    fn label(self) -> &'static str {
        match self {
            AuditVerdict::Safe => "SAFE",
            AuditVerdict::Suspicious => "SUSPICIOUS",
            AuditVerdict::Unsafe => "UNSAFE",
        }
    }
}

pub async fn run(cli: Cli) -> anyhow::Result<String> {
    let package_name = cli
        .package_name
        .as_deref()
        .context("missing package name; use `aur-audit <package>` or `aur-audit config ...`")?;
    let saved_config = config::load_config().context("failed to load config")?;
    let provider = cli
        .provider
        .or(saved_config.provider)
        .unwrap_or(crate::cli::Provider::Openrouter);
    let model = cli.model.clone().or(saved_config.model);
    let base_url = cli.base_url.clone().or(saved_config.base_url);
    let api_key = cli.api_key.clone().or(saved_config.api_key);
    let config = ProviderConfig::from_options(provider, model, base_url, api_key);
    let ai = AiClient::new(config);

    let cloned = if let Some(local_repo) = &cli.local_repo {
        if cli.verbose {
            eprintln!("Auditing local repository at {}", local_repo.display());
        }
        None
    } else {
        Some(crate::aur::clone_aur_package(package_name, cli.verbose)?)
    };
    let repo_path = cli
        .local_repo
        .clone()
        .unwrap_or_else(|| cloned.as_ref().expect("clone exists").path().to_path_buf());

    let inventory = build_inventory(&repo_path).context("failed to build repository inventory")?;
    let first_prompt = build_first_prompt(package_name, &inventory);
    if cli.verbose {
        eprintln!(
            "Sending complete inventory with {} items to AI",
            inventory.items.len()
        );
    }

    let selection_answer = ai.chat(&first_prompt).await?;
    if cli.verbose {
        eprintln!("AI file selection:\n{selection_answer}");
    }

    let selection = build_file_selection(&selection_answer, &inventory);
    if cli.verbose {
        eprintln!("Opening {} selected files", selection.paths.len());
    }

    let selected_files = read_selected_files(
        &repo_path,
        &inventory,
        &selection.paths,
        cli.max_file_bytes,
        cli.max_total_bytes,
    )
    .context("failed to read selected files")?;

    let second_prompt = build_second_prompt(
        package_name,
        &inventory,
        &selection.paths,
        &selection.ai_requested_paths,
        &format_selected_files_for_prompt(&selected_files),
        &selected_files.truncated_or_skipped,
    );

    println!(
        "\n{}\n{}",
        format!("aur-audit: scanning {package_name}").bold(),
        "Streaming final AI review...".dimmed()
    );
    let final_report = ai.chat_stream_stdout(&second_prompt).await?;
    if !is_plain_text_report(&final_report) {
        bail!("AI returned JSON or tool-call-like output instead of a plain text report");
    }

    if cli.keep_temp && cli.local_repo.is_some() {
        eprintln!("--keep-temp has no effect with --local-repo");
    } else if cli.keep_temp {
        let cloned = cloned.expect("clone exists");
        let kept_path = cloned.keep()?;
        eprintln!("Kept temporary clone at {}", kept_path.display());
    }

    let verdict = parse_audit_verdict(&final_report)
        .context("AI report did not contain a final Verdict line; refusing successful exit")?;
    if verdict.is_blocking() {
        bail!(
            "aur-audit verdict is {}; refusing successful exit",
            verdict.label()
        );
    }

    Ok(String::new())
}

fn parse_audit_verdict(report: &str) -> Option<AuditVerdict> {
    for line in report.lines() {
        let normalized = line
            .trim()
            .trim_matches(|ch| ch == '*' || ch == '`')
            .to_ascii_uppercase();
        let Some(candidate) = normalized.strip_prefix("VERDICT:").map(str::trim) else {
            continue;
        };

        if candidate.starts_with("UNSAFE") {
            return Some(AuditVerdict::Unsafe);
        }
        if candidate.starts_with("SUSPICIOUS") {
            return Some(AuditVerdict::Suspicious);
        }
        if candidate.starts_with("SAFE") {
            return Some(AuditVerdict::Safe);
        }
    }

    None
}

pub fn build_first_prompt(package_name: &str, inventory: &Inventory) -> String {
    format!(
        r#"You are auditing an Arch Linux AUR package.

Package name: {package_name}

You have not seen file contents yet. You are only seeing the complete repository inventory.

Your job is to decide which files must be opened for security review.

Do not restrict yourself to common AUR files. Any file can matter. Look for unusual names, hidden files, scripts, patches, install hooks, service files, config files, files with no extension, generated-looking files, vendored code, symlinks, binary blobs, and files that may affect build or install behavior.

Reply in plain text only.

Write a section called:

FILES TO OPEN

Under it, list one file path per line.

After each file path, add a short reason.

Example:

FILES TO OPEN
PKGBUILD - main build instructions
.install - install-time behavior
weirdfile - unknown file with no extension
.hidden/script - hidden script that may affect build

Do not return JSON.
Do not use tool calls.
Do not write code.
Only list files that should be opened and explain briefly why.

Complete repository inventory:

{}"#,
        inventory.format_for_prompt()
    )
}

pub fn build_second_prompt(
    package_name: &str,
    inventory: &Inventory,
    selected_paths: &[String],
    ai_requested_paths: &[String],
    selected_file_contents: &str,
    truncated_or_skipped: &[String],
) -> String {
    let selected_paths_text = selected_paths.join("\n");
    let ai_requested_text = if ai_requested_paths.is_empty() {
        "(AI did not select any existing files; mandatory minimum files were added if present)"
            .to_string()
    } else {
        ai_requested_paths.join("\n")
    };
    let limits_text = if truncated_or_skipped.is_empty() {
        "(none)".to_string()
    } else {
        truncated_or_skipped.join("\n")
    };

    format!(
        r#"You are auditing an Arch Linux AUR package.

Package name: {package_name}

You are now seeing:
- the complete repository inventory
- the files you selected for inspection
- readable contents of selected text files
- metadata for selected binary or truncated files

Do not assume the package is safe.
Do not execute code.
Review only the provided repository inventory and file contents.

Focus on:
- malicious build or install behavior
- unexpected downloads
- hidden scripts
- persistence
- service activation
- privilege escalation
- credential access
- exfiltration
- obfuscation
- unpinned or mutable sources
- suspicious binaries
- suspicious symlinks
- files that should have been reviewed but were unavailable

Severity calibration:
- Do not inflate severity just because something is unusual.
- SAFE is acceptable when reviewed behavior matches the package purpose and there is no concrete evidence of malicious behavior.
- SUSPICIOUS should require a meaningful unresolved risk, such as unverifiable binaries, skipped integrity checks, hidden execution paths, install-time system changes, obfuscation, or behavior that does not match the package purpose.
- UNSAFE should require clear malicious behavior or very high-risk behavior, such as credential theft, persistence without clear user consent, privilege escalation, destructive actions, or exfiltration.
- A service or localhost-only server is not automatically suspicious. Treat it as LOW or informational unless it binds non-local interfaces, lacks expected access controls, executes attacker-controlled input, persists unexpectedly, or does not fit the package purpose.
- A dependency that may handle credentials is not automatically suspicious. Recommend reviewing that dependency separately, but do not count it as a finding unless this package misuses it or changes its behavior.
- Downloaded sources with fixed cryptographic checksums are not "mutable" for this package review unless the checksum is missing, skipped, weak, or inconsistent. You may still mention that upstream compromise is a general residual risk.
- Patching bundled or minified files is fragile, but it is not malicious by itself. Judge whether the patch behavior is understandable, scoped, and consistent with the package purpose.
- Report concrete evidence from the provided files. Put general caveats in Summary or Skipped, binary, or truncated files, not as medium/high findings.

Write a plain English security report.

Use this exact output order:

Files scanned:
For each file opened for review, output one line:
- GOOD, SUSPICIOUS, or UNSAFE
- file path
- one short evidence-based reason

Then output a blank line and this full report:

aur-audit: <package-name>

Verdict: SAFE, SUSPICIOUS, or UNSAFE

Confidence: LOW, MEDIUM, or HIGH

Summary:
Explain the overall result in a few sentences.

Files reviewed:
List the files reviewed and why they mattered.

Findings:
For each finding, include:
- severity: LOW, MEDIUM, HIGH, or CRITICAL
- file path
- line number if available
- what is suspicious or safe
- why it matters

Skipped, binary, or truncated files:
Explain anything that limited the review.

Recommended action:
Say one of:
- Safe to build
- Build only in a clean chroot or container
- Manual review required
- Do not install

Complete repository inventory:

{}

Files the AI selected:

{ai_requested_text}

Files opened for review, including mandatory minimum files if present:

{selected_paths_text}

Selected file contents and metadata:
{selected_file_contents}

Skipped, binary, or truncated files:

{limits_text}
"#,
        inventory.format_for_prompt()
    )
}

pub fn is_plain_text_report(report: &str) -> bool {
    let trimmed = report.trim();
    !(trimmed.starts_with('{') || trimmed.starts_with('['))
        && !trimmed.contains("\"tool_calls\"")
        && !trimmed.contains("\"function_call\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Inventory;

    #[test]
    fn final_report_plain_text_check_rejects_json() {
        assert!(is_plain_text_report(
            "aur-audit: x\n\nVerdict: SAFE\n\nSummary:\nAI review cannot prove safety."
        ));
        assert!(!is_plain_text_report("{\"verdict\":\"SAFE\"}"));
        assert!(!is_plain_text_report("{\"tool_calls\":[]}"));
    }

    #[test]
    fn final_prompt_contains_severity_calibration() {
        let inventory = Inventory { items: Vec::new() };
        let prompt = build_second_prompt("pkg", &inventory, &[], &[], "", &[]);

        assert!(prompt.contains("Severity calibration:"));
        assert!(
            prompt.contains("A service or localhost-only server is not automatically suspicious")
        );
        assert!(prompt
            .contains("Downloaded sources with fixed cryptographic checksums are not \"mutable\""));
        assert!(prompt.contains("SAFE is acceptable"));
        assert!(!prompt.contains("Consistency rule:"));
        assert!(!prompt.contains("Preliminary per-file AI scan verdicts:"));
        assert!(prompt.contains("Files scanned:"));
        assert!(prompt.contains("Then output a blank line and this full report:"));
    }

    #[test]
    fn parses_audit_verdicts_for_exit_status() {
        assert_eq!(
            parse_audit_verdict("aur-audit: x\n\nVerdict: SAFE\n"),
            Some(AuditVerdict::Safe)
        );
        assert_eq!(
            parse_audit_verdict("aur-audit: x\n\nVerdict: SUSPICIOUS\n"),
            Some(AuditVerdict::Suspicious)
        );
        assert_eq!(
            parse_audit_verdict("aur-audit: x\n\nVerdict: UNSAFE\n"),
            Some(AuditVerdict::Unsafe)
        );
        assert_eq!(parse_audit_verdict("aur-audit: x\n"), None);
    }

    #[test]
    fn only_safe_verdict_allows_successful_exit() {
        assert!(!AuditVerdict::Safe.is_blocking());
        assert!(AuditVerdict::Suspicious.is_blocking());
        assert!(AuditVerdict::Unsafe.is_blocking());
    }
}
