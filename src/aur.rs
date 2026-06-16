use anyhow::{bail, Context};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub struct ClonedRepo {
    temp_dir: Option<TempDir>,
    repo_path: PathBuf,
}

impl ClonedRepo {
    pub fn path(&self) -> &Path {
        &self.repo_path
    }

    pub fn keep(mut self) -> anyhow::Result<PathBuf> {
        let temp_dir = self
            .temp_dir
            .take()
            .context("temporary directory was already consumed")?;
        let path = temp_dir.keep();
        Ok(path)
    }
}

pub fn clone_aur_package(package_name: &str, verbose: bool) -> anyhow::Result<ClonedRepo> {
    validate_package_name(package_name)?;

    let temp_dir = tempfile::Builder::new()
        .prefix("aur-audit-")
        .tempdir()
        .context("failed to create temporary directory")?;
    let repo_path = temp_dir.path().join(package_name);
    let url = format!("https://aur.archlinux.org/{package_name}.git");

    if verbose {
        eprintln!("Cloning {url} into {}", repo_path.display());
    }

    let output = Command::new("git")
        .arg("-c")
        .arg("protocol.file.allow=never")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--no-tags")
        .arg(&url)
        .arg(&repo_path)
        .output()
        .context("failed to start git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git clone failed: {}", stderr.trim());
    }

    Ok(ClonedRepo {
        temp_dir: Some(temp_dir),
        repo_path,
    })
}

pub fn validate_package_name(package_name: &str) -> anyhow::Result<()> {
    if package_name.is_empty() {
        bail!("package name must not be empty");
    }

    let valid = package_name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'@' | b'.' | b'_' | b'+' | b'-'));

    if !valid || package_name == "." || package_name == ".." || package_name.contains("..") {
        bail!("invalid AUR package name: {package_name}");
    }

    Ok(())
}
