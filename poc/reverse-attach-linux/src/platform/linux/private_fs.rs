//! Owner-only files for secrets (grants): directories 0700, files 0600, and a
//! file found wider than 0600 is narrowed back before it is trusted.

use std::fs;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// `$XDG_STATE_HOME`, else `~/.local/state`.
pub(crate) fn state_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
}

pub(crate) fn create_dir_all(dir: &Path) -> std::io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// A new file only the owner can read or write; fails if `path` exists.
pub(crate) fn create_new(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// If something widened `path`, narrow it back to 0600 and say so.
pub(crate) fn narrow(path: &Path, meta: &fs::Metadata) -> Result<(), String> {
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        eprintln!(
            "grants: {} was mode {mode:o}; resetting to 600",
            path.display()
        );
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    Ok(())
}
