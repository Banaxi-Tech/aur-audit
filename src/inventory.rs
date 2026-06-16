use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoItemType {
    File,
    Directory,
    Symlink,
    Other,
}

impl std::fmt::Display for RepoItemType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            RepoItemType::File => "file",
            RepoItemType::Directory => "directory",
            RepoItemType::Symlink => "symlink",
            RepoItemType::Other => "other",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone)]
pub struct InventoryItem {
    pub relative_path: String,
    pub item_type: RepoItemType,
    pub size: u64,
    pub is_text: Option<bool>,
    pub is_hidden: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    pub symlink_points_outside_repo: bool,
    pub extension: Option<String>,
    pub first_line: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Inventory {
    pub items: Vec<InventoryItem>,
}

impl Inventory {
    pub fn item(&self, path: &str) -> Option<&InventoryItem> {
        self.items.iter().find(|item| item.relative_path == path)
    }

    pub fn paths(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|item| item.relative_path.clone())
            .collect()
    }

    pub fn format_for_prompt(&self) -> String {
        let mut out = String::new();
        for item in &self.items {
            let text_kind = match item.is_text {
                Some(true) => "text",
                Some(false) => "binary",
                None => "n/a",
            };
            let extension = item.extension.as_deref().unwrap_or("(none)");
            let target = item.symlink_target.as_deref().unwrap_or("");
            let outside = if item.symlink_points_outside_repo {
                "yes"
            } else {
                "no"
            };
            let first_line = item.first_line.as_deref().unwrap_or("");
            let sha256 = item.sha256.as_deref().unwrap_or("n/a");

            out.push_str(&format!(
                "- path: {}\n  type: {}\n  size: {}\n  text_or_binary: {}\n  hidden: {}\n  symlink: {}\n  symlink_target: {}\n  symlink_points_outside_repo: {}\n  extension: {}\n  first_line_or_shebang: {}\n  sha256: {}\n",
                item.relative_path,
                item.item_type,
                item.size,
                text_kind,
                item.is_hidden,
                item.is_symlink,
                target,
                outside,
                extension,
                sanitize_line(first_line),
                sha256
            ));
        }
        out
    }
}

pub fn build_inventory(repo_root: &Path) -> Result<Inventory> {
    let mut items = Vec::new();

    for entry in WalkDir::new(repo_root).follow_links(false).min_depth(1) {
        let entry = entry.context("failed to walk repository")?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("failed to read metadata for {}", path.display()))?;
        let relative_path = path
            .strip_prefix(repo_root)
            .context("walked path outside repository")?
            .to_string_lossy()
            .replace('\\', "/");

        let file_type = metadata.file_type();
        let is_symlink = file_type.is_symlink();
        let item_type = if is_symlink {
            RepoItemType::Symlink
        } else if file_type.is_file() {
            RepoItemType::File
        } else if file_type.is_dir() {
            RepoItemType::Directory
        } else {
            RepoItemType::Other
        };

        let symlink_target_path =
            if is_symlink {
                Some(fs::read_link(path).with_context(|| {
                    format!("failed to read symlink target for {}", path.display())
                })?)
            } else {
                None
            };
        let symlink_target = symlink_target_path
            .as_ref()
            .map(|target| target.to_string_lossy().to_string());
        let symlink_points_outside_repo = symlink_target_path
            .as_ref()
            .map(|target| symlink_points_outside(repo_root, path, target))
            .unwrap_or(false);

        let is_regular_file = item_type == RepoItemType::File;
        let is_text = if is_regular_file {
            Some(file_looks_text(path)?)
        } else {
            None
        };
        let first_line = if is_regular_file && is_text == Some(true) {
            read_first_line(path).ok()
        } else {
            None
        };
        let sha256 = if is_regular_file {
            Some(sha256_file(path)?)
        } else if let Some(target) = &symlink_target {
            Some(sha256_bytes(target.as_bytes()))
        } else {
            None
        };

        items.push(InventoryItem {
            relative_path: relative_path.clone(),
            item_type,
            size: metadata.len(),
            is_text,
            is_hidden: is_hidden_path(Path::new(&relative_path)),
            is_symlink,
            symlink_target,
            symlink_points_outside_repo,
            extension: extension(Path::new(&relative_path)),
            first_line,
            sha256,
        });
    }

    items.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(Inventory { items })
}

pub fn file_looks_text(path: &Path) -> Result<bool> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for text detection: {}", path.display()))?;
    let mut buffer = [0_u8; 8192];
    let n = file
        .read(&mut buffer)
        .with_context(|| format!("failed to read file for text detection: {}", path.display()))?;
    let sample = &buffer[..n];
    if sample.contains(&0) {
        return Ok(false);
    }
    Ok(std::str::from_utf8(sample).is_ok())
}

fn read_first_line(path: &Path) -> Result<String> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read first line from {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let line = text.lines().next().unwrap_or("").to_string();
    Ok(line.chars().take(240).collect())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for hashing: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buffer)
            .with_context(|| format!("failed to hash {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn is_hidden_path(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().starts_with('.'))
    })
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(OsStr::to_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn symlink_points_outside(repo_root: &Path, link_path: &Path, target: &Path) -> bool {
    let resolved = if target.is_absolute() {
        target.to_path_buf()
    } else {
        link_path.parent().unwrap_or(repo_root).join(target)
    };
    !normalize_lexically(&resolved).starts_with(&normalize_lexically(repo_root))
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn sanitize_line(line: &str) -> String {
    line.replace('\n', "\\n").replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[test]
    fn complete_inventory_includes_all_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::create_dir(dir.path().join("sub"))?;
        fs::write(dir.path().join("PKGBUILD"), "pkgname=x\n")?;
        fs::write(dir.path().join("sub").join("data.txt"), "data\n")?;

        let inventory = build_inventory(dir.path())?;

        assert!(inventory.item("PKGBUILD").is_some());
        assert!(inventory.item("sub").is_some());
        assert!(inventory.item("sub/data.txt").is_some());
        Ok(())
    }

    #[test]
    fn hidden_files_and_no_extension_files_are_included() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::create_dir(dir.path().join(".hidden"))?;
        fs::write(dir.path().join(".hidden").join("script"), "#!/bin/sh\n")?;
        fs::write(dir.path().join("noextension"), "plain\n")?;

        let inventory = build_inventory(dir.path())?;
        let hidden = inventory.item(".hidden/script").expect("hidden script");
        let no_extension = inventory.item("noextension").expect("no extension file");

        assert!(hidden.is_hidden);
        assert_eq!(hidden.extension, None);
        assert_eq!(no_extension.extension, None);
        Ok(())
    }

    #[test]
    fn binary_files_are_detected() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("blob.bin"), [0, 159, 146, 150])?;

        let inventory = build_inventory(dir.path())?;
        let item = inventory.item("blob.bin").expect("binary item");

        assert_eq!(item.is_text, Some(false));
        assert!(item.sha256.is_some());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_outside_repo_are_detected_and_not_followed() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let outside = tempfile::NamedTempFile::new()?;
        symlink(outside.path(), dir.path().join("outside-link"))?;

        let inventory = build_inventory(dir.path())?;
        let item = inventory.item("outside-link").expect("symlink item");

        assert!(item.is_symlink);
        assert!(item.symlink_points_outside_repo);
        assert_eq!(item.is_text, None);
        Ok(())
    }
}
