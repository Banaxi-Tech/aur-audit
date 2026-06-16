use crate::inventory::{Inventory, RepoItemType};
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub enum SelectedFileData {
    Text {
        path: String,
        content: String,
        truncated: bool,
        original_size: u64,
        included_bytes: usize,
        reason: String,
    },
    Binary {
        path: String,
        size: u64,
        sha256: Option<String>,
        reason: String,
    },
    Symlink {
        path: String,
        target: Option<String>,
        points_outside_repo: bool,
        sha256: Option<String>,
        reason: String,
    },
    Skipped {
        path: String,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct FileSelection {
    pub paths: Vec<String>,
    pub ai_requested_paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ReadSelectedFiles {
    pub files: Vec<SelectedFileData>,
    pub truncated_or_skipped: Vec<String>,
}

pub fn extract_ai_requested_paths(answer: &str, inventory: &Inventory) -> Vec<String> {
    let known_paths = inventory.paths();
    let mut sorted_paths = known_paths.clone();
    sorted_paths.sort_by_key(|path| std::cmp::Reverse(path.len()));

    let mut in_section = false;
    let mut selected = BTreeSet::new();

    for raw_line in answer.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.eq_ignore_ascii_case("FILES TO OPEN")
            || trimmed.eq_ignore_ascii_case("FILES TO OPEN:")
        {
            in_section = true;
            continue;
        }

        if !in_section {
            continue;
        }

        if let Some(path) = extract_path_from_line(trimmed, &known_paths, &sorted_paths) {
            selected.insert(path);
        }
    }

    selected.into_iter().collect()
}

pub fn build_file_selection(ai_answer: &str, inventory: &Inventory) -> FileSelection {
    let ai_requested_paths = extract_ai_requested_paths(ai_answer, inventory);
    let mut selected: BTreeSet<String> = ai_requested_paths.iter().cloned().collect();

    for mandatory in mandatory_paths(inventory) {
        selected.insert(mandatory);
    }

    FileSelection {
        paths: selected.into_iter().collect(),
        ai_requested_paths,
    }
}

pub fn mandatory_paths(inventory: &Inventory) -> Vec<String> {
    inventory
        .items
        .iter()
        .filter(|item| {
            item.relative_path == "PKGBUILD"
                || item.relative_path == ".SRCINFO"
                || item.relative_path.ends_with(".install")
        })
        .map(|item| item.relative_path.clone())
        .collect()
}

pub fn read_selected_files(
    repo_root: &Path,
    inventory: &Inventory,
    selected_paths: &[String],
    max_file_bytes: usize,
    max_total_bytes: usize,
) -> Result<ReadSelectedFiles> {
    if max_file_bytes == 0 {
        bail!("--max-file-bytes must be greater than 0");
    }

    let mut remaining_total = max_total_bytes;
    let mut files = Vec::new();
    let mut truncated_or_skipped = Vec::new();

    for path in selected_paths {
        let Some(item) = inventory.item(path) else {
            let reason = "requested path was not found in inventory".to_string();
            truncated_or_skipped.push(format!("{path}: {reason}"));
            files.push(SelectedFileData::Skipped {
                path: path.clone(),
                reason,
            });
            continue;
        };

        match item.item_type {
            RepoItemType::Symlink => {
                let reason = if item.symlink_points_outside_repo {
                    "symlink was not followed; it points outside the cloned repository".to_string()
                } else {
                    "symlink was not followed; symlinks are treated as hostile metadata".to_string()
                };
                truncated_or_skipped.push(format!("{path}: {reason}"));
                files.push(SelectedFileData::Symlink {
                    path: path.clone(),
                    target: item.symlink_target.clone(),
                    points_outside_repo: item.symlink_points_outside_repo,
                    sha256: item.sha256.clone(),
                    reason,
                });
            }
            RepoItemType::Directory | RepoItemType::Other => {
                let reason = format!("{} item has no readable file contents", item.item_type);
                truncated_or_skipped.push(format!("{path}: {reason}"));
                files.push(SelectedFileData::Skipped {
                    path: path.clone(),
                    reason,
                });
            }
            RepoItemType::File => {
                if item.is_text != Some(true) {
                    files.push(SelectedFileData::Binary {
                        path: path.clone(),
                        size: item.size,
                        sha256: item.sha256.clone(),
                        reason: "binary file; contents were not sent to the AI".to_string(),
                    });
                    truncated_or_skipped.push(format!(
                        "{path}: binary file, contents omitted; size={}, sha256={}",
                        item.size,
                        item.sha256.as_deref().unwrap_or("n/a")
                    ));
                    continue;
                }

                if remaining_total == 0 {
                    let reason =
                        "skipped because the selected file content byte budget was exhausted"
                            .to_string();
                    truncated_or_skipped.push(format!("{path}: {reason}"));
                    files.push(SelectedFileData::Skipped {
                        path: path.clone(),
                        reason,
                    });
                    continue;
                }

                let full_path = repo_root.join(path);
                let metadata = fs::symlink_metadata(&full_path).with_context(|| {
                    format!(
                        "failed to re-check selected file metadata: {}",
                        full_path.display()
                    )
                })?;
                if metadata.file_type().is_symlink() {
                    let reason =
                        "selected path became a symlink before reading; not followed".to_string();
                    truncated_or_skipped.push(format!("{path}: {reason}"));
                    files.push(SelectedFileData::Skipped {
                        path: path.clone(),
                        reason,
                    });
                    continue;
                }

                let bytes = fs::read(&full_path).with_context(|| {
                    format!("failed to read selected file {}", full_path.display())
                })?;
                let per_file_limit = max_file_bytes.min(remaining_total);
                let truncated = bytes.len() > per_file_limit;
                let included = if truncated {
                    truncate_bytes_beginning_and_end(&bytes, per_file_limit)
                } else {
                    bytes
                };
                remaining_total = remaining_total.saturating_sub(included.len());
                let content = String::from_utf8_lossy(&included).to_string();

                if truncated {
                    let warning = format!(
                        "{path}: truncated from {} bytes to {} bytes",
                        item.size,
                        content.len()
                    );
                    truncated_or_skipped.push(warning);
                }

                files.push(SelectedFileData::Text {
                    path: path.clone(),
                    content,
                    truncated,
                    original_size: item.size,
                    included_bytes: per_file_limit.min(item.size as usize),
                    reason: "selected for AI review".to_string(),
                });
            }
        }
    }

    Ok(ReadSelectedFiles {
        files,
        truncated_or_skipped,
    })
}

pub fn format_selected_files_for_prompt(read: &ReadSelectedFiles) -> String {
    let mut out = String::new();
    for file in &read.files {
        match file {
            SelectedFileData::Text {
                path,
                content,
                truncated,
                original_size,
                included_bytes,
                reason,
            } => {
                out.push_str(&format!(
                    "\n=== FILE: {path} ===\nreason: {reason}\ntext: yes\ntruncated: {truncated}\noriginal_size: {original_size}\nincluded_bytes: {included_bytes}\n--- BEGIN CONTENT ---\n{content}\n--- END CONTENT ---\n"
                ));
            }
            SelectedFileData::Binary {
                path,
                size,
                sha256,
                reason,
            } => {
                out.push_str(&format!(
                    "\n=== FILE: {path} ===\nreason: {reason}\ntext: no\nbinary: yes\nsize: {size}\nsha256: {}\ncontents: omitted\n",
                    sha256.as_deref().unwrap_or("n/a")
                ));
            }
            SelectedFileData::Symlink {
                path,
                target,
                points_outside_repo,
                sha256,
                reason,
            } => {
                out.push_str(&format!(
                    "\n=== FILE: {path} ===\nreason: {reason}\ntype: symlink\nsymlink_target: {}\nsymlink_points_outside_repo: {points_outside_repo}\nsha256: {}\ncontents: not followed\n",
                    target.as_deref().unwrap_or(""),
                    sha256.as_deref().unwrap_or("n/a")
                ));
            }
            SelectedFileData::Skipped { path, reason } => {
                out.push_str(&format!(
                    "\n=== FILE: {path} ===\nskipped: yes\nreason: {reason}\n"
                ));
            }
        }
    }
    out
}

fn extract_path_from_line(
    line: &str,
    known_paths: &[String],
    sorted_paths: &[String],
) -> Option<String> {
    let cleaned = strip_line_prefix(line);
    let cleaned = cleaned.trim_matches(|c| c == '`' || c == '\'' || c == '"');

    if known_paths.iter().any(|path| path == cleaned) {
        return Some(cleaned.to_string());
    }

    for separator in [" - ", " -- ", " – ", " — ", ": ", "\t"] {
        if let Some((candidate, _)) = cleaned.split_once(separator) {
            let candidate = candidate
                .trim()
                .trim_matches(|c| c == '`' || c == '\'' || c == '"');
            if known_paths.iter().any(|path| path == candidate) {
                return Some(candidate.to_string());
            }
        }
    }

    for path in sorted_paths {
        if cleaned == path {
            return Some(path.clone());
        }
        if let Some(rest) = cleaned.strip_prefix(path) {
            let next = rest.chars().next();
            if next.is_none() || matches!(next, Some(' ' | '\t' | '-' | ':' | '–' | '—')) {
                return Some(path.clone());
            }
        }
    }

    None
}

fn strip_line_prefix(line: &str) -> &str {
    let mut value = line.trim();
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = value.strip_prefix(prefix) {
            value = rest.trim();
        }
    }

    let mut chars = value.char_indices();
    let mut digit_end = None;
    for (idx, ch) in &mut chars {
        if ch.is_ascii_digit() {
            digit_end = Some(idx + ch.len_utf8());
            continue;
        }
        if matches!(ch, '.' | ')') {
            if let Some(end) = digit_end {
                value = value[end + ch.len_utf8()..].trim();
            }
        }
        break;
    }

    value
}

fn truncate_bytes_beginning_and_end(bytes: &[u8], limit: usize) -> Vec<u8> {
    const MARKER: &[u8] = b"\n\n[aur-audit: file truncated; middle omitted]\n\n";
    if limit <= MARKER.len() + 2 {
        return bytes[..limit.min(bytes.len())].to_vec();
    }
    let side = (limit - MARKER.len()) / 2;
    let mut out = Vec::with_capacity(limit);
    out.extend_from_slice(&bytes[..side]);
    out.extend_from_slice(MARKER);
    out.extend_from_slice(&bytes[bytes.len() - side..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::build_inventory;
    use std::io::Write;

    #[test]
    fn parses_plain_text_file_list() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("PKGBUILD"), "pkgname=x\n")?;
        fs::write(dir.path().join("weirdfile"), "hello\n")?;
        let inventory = build_inventory(dir.path())?;
        let answer = "Notes\n\nFILES TO OPEN\nPKGBUILD - build instructions\nweirdfile - no extension\nmissing - no\n";

        let paths = extract_ai_requested_paths(answer, &inventory);
        assert_eq!(paths, vec!["PKGBUILD", "weirdfile"]);
        Ok(())
    }

    #[test]
    fn mandatory_minimum_files_are_added() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("PKGBUILD"), "pkgname=x\n")?;
        fs::write(dir.path().join(".SRCINFO"), "pkgbase = x\n")?;
        fs::write(dir.path().join("x.install"), "post_install() { :; }\n")?;
        fs::write(dir.path().join("extra"), "x\n")?;
        let inventory = build_inventory(dir.path())?;

        let selection = build_file_selection("FILES TO OPEN\nextra - reason\n", &inventory);
        assert!(selection.paths.contains(&"PKGBUILD".to_string()));
        assert!(selection.paths.contains(&".SRCINFO".to_string()));
        assert!(selection.paths.contains(&"x.install".to_string()));
        assert!(selection.paths.contains(&"extra".to_string()));
        Ok(())
    }

    #[test]
    fn large_selected_files_are_truncated_with_warning() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut file = fs::File::create(dir.path().join("large.txt"))?;
        for _ in 0..200 {
            writeln!(file, "0123456789")?;
        }
        let inventory = build_inventory(dir.path())?;
        let read = read_selected_files(dir.path(), &inventory, &["large.txt".to_string()], 80, 80)?;

        assert_eq!(read.files.len(), 1);
        assert!(read.truncated_or_skipped[0].contains("truncated"));
        match &read.files[0] {
            SelectedFileData::Text {
                truncated, content, ..
            } => {
                assert!(*truncated);
                assert!(content.contains("file truncated"));
            }
            _ => panic!("expected text file"),
        }
        Ok(())
    }
}
