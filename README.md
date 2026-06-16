# aur-audit

AI-assisted security review for Arch User Repository packages.

`aur-audit` downloads or reads an AUR package directory, inventories the full
repository, asks an AI model which files need review, opens those files as data,
and streams a final security report. It is designed to make suspicious AUR build
logic easier to notice before you run `makepkg` or install a package.

It does not execute package code.

## Features

- Reviews the full repository inventory, not only `PKGBUILD`.
- Includes hidden files, install hooks, service units, patches, symlinks, binary
  metadata, files with no extension, and generated-looking scripts in the
  initial inventory.
- Opens AI-selected files plus mandatory package control files when present:
  `PKGBUILD`, `.SRCINFO`, and `*.install`.
- Streams the final model response so file verdicts and the report appear as the
  model writes them.
- Uses temperature `0`.
- Supports OpenRouter, Ollama, and LM Studio.
- Supports persistent config so normal use can be just `aur-audit <package>`.
- Exits non-zero for `SUSPICIOUS`, `UNSAFE`, or malformed reports, so it can be
  used in shell chains.

## Install

From the repository:

```bash
cargo install --path .
```

The binary is installed to Cargo's bin directory, usually:

```text
~/.cargo/bin/aur-audit
```

Make sure that directory is in `PATH`.

## Quick Start

Configure a provider once:

```bash
aur-audit config set --provider lmstudio --model gemma-4-12b-it-qat --base-url http://localhost:1234/v1
```

Then audit packages by name:

```bash
aur-audit google-chrome
```

Audit a local package checkout:

```bash
aur-audit google-chrome --local-repo ./google-chrome
```

Use it as a gate before installing:

```bash
aur-audit bterminal && yay -S bterminal
```

`yay` runs only if the final verdict is `SAFE`.

## Exit Codes

`aur-audit` uses the final report verdict as the command result.

| Final verdict | Exit code | Meaning |
| --- | ---: | --- |
| `SAFE` | `0` | Shell chains may continue. |
| `SUSPICIOUS` | non-zero | Manual review required. |
| `UNSAFE` | non-zero | Do not install. |
| Missing or invalid verdict | non-zero | The report was not trusted. |

Example:

```bash
aur-audit package-name && yay -S package-name
```

## Configuration

Config path:

```bash
aur-audit config path
```

Default path:

```text
~/.config/aur-audit/config
```

Show current config:

```bash
aur-audit config show
```

Set LM Studio defaults:

```bash
aur-audit config set \
  --provider lmstudio \
  --model gemma-4-12b-it-qat \
  --base-url http://localhost:1234/v1
```

Set OpenRouter defaults:

```bash
aur-audit config set \
  --provider openrouter \
  --model openai/gpt-5.5 \
  --api-key sk-or-...
```

Remove a stored API key:

```bash
aur-audit config set --clear-api-key
```

Config file format:

```text
provider=lmstudio
model=gemma-4-12b-it-qat
base_url=http://localhost:1234/v1
api_key=sk-or-...
```

CLI flags override config values for one run.

## Usage

```bash
aur-audit [OPTIONS] [PACKAGE_NAME] [COMMAND]
```

Common scan options:

```bash
aur-audit <package-name>
aur-audit <package-name> --provider lmstudio --model gemma-4-12b-it-qat
aur-audit <package-name> --provider openrouter --model openai/gpt-5.5 --api-key sk-or-...
aur-audit <package-name> --provider ollama --model qwen3.5-coder
aur-audit <package-name> --base-url http://localhost:1234/v1
aur-audit <package-name> --local-repo ./package-dir
aur-audit <package-name> --keep-temp
aur-audit <package-name> --verbose
aur-audit <package-name> --max-file-bytes 1048576
aur-audit <package-name> --max-total-bytes 800000
```

Config commands:

```bash
aur-audit config set [OPTIONS]
aur-audit config show
aur-audit config path
```

## Providers

### LM Studio

Default endpoint:

```text
http://localhost:1234/v1/chat/completions
```

Recommended setup:

```bash
aur-audit config set --provider lmstudio --model gemma-4-12b-it-qat --base-url http://localhost:1234/v1
```

Start the LM Studio local server and load the configured model before scanning.

### Ollama

Default endpoint:

```text
http://localhost:11434/api/chat
```

Example:

```bash
aur-audit config set --provider ollama --model qwen3.5-coder --base-url http://localhost:11434
```

### OpenRouter

Default endpoint:

```text
https://openrouter.ai/api/v1/chat/completions
```

Set a key in config:

```bash
aur-audit config set --provider openrouter --model openai/gpt-5.5 --api-key sk-or-...
```

Or use the environment:

```bash
export OPENROUTER_API_KEY=sk-or-...
aur-audit google-chrome --provider openrouter --model openai/gpt-5.5
```

Provider base URLs can also be supplied through environment variables:

```bash
AUR_AUDIT_LMSTUDIO_BASE_URL=http://localhost:1234/v1
AUR_AUDIT_OLLAMA_BASE_URL=http://localhost:11434
AUR_AUDIT_OPENROUTER_BASE_URL=https://openrouter.ai/api/v1
```

## How It Works

`aur-audit` performs two model requests:

1. Inventory pass:
   The model receives a complete repository inventory and selects files that
   need inspection.
2. Review pass:
   `aur-audit` reads the selected files as data and streams one final response.
   The response starts with per-file status lines, then prints the full report.

The final report format includes:

```text
Files scanned:
GOOD, PKGBUILD - reason
SUSPICIOUS, package.install - reason

aur-audit: <package>

Verdict: SAFE, SUSPICIOUS, or UNSAFE

Confidence: LOW, MEDIUM, or HIGH

Summary:
...

Files reviewed:
...

Findings:
...

Skipped, binary, or truncated files:
...

Recommended action:
...
```

## Safety Model

`aur-audit` treats AUR repositories as hostile input.

It does not:

- run `makepkg`
- source `PKGBUILD`
- execute shell scripts
- execute install hooks
- install dependencies
- follow symlinks outside the repository
- write outside the temporary clone, except when preserving a clone with
  `--keep-temp`

Files are read as data. Binary files are represented by metadata such as path,
size, and SHA256. Large text files are truncated with beginning and ending
sections preserved.

## AUR Helper Integration

`yay` does not provide a plugin system or a pre-build hook API for automatically
running external scanners.

Use shell chaining instead:

```bash
aur-audit package-name && yay -S package-name
```

For a local review/install flow:

```bash
tmp="$(mktemp -d)"
cd "$tmp"
yay -G package-name
aur-audit package-name --local-repo "$tmp/package-name" && yay -Bi "$tmp/package-name"
```

## Limitations

AI review is not a proof of safety. It can miss malicious behavior, misunderstand
shell logic, fail to decode obfuscation, or overstate benign behavior. Treat the
result as a review aid.

Use clean chroots, containers, and manual review for packages that touch install
hooks, systemd units, cron directories, desktop autostart entries, shell startup
files, package manager hooks, bundled binaries, obfuscated strings, mutable
sources, or network-executed commands.

