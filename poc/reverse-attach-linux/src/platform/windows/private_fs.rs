//! Owner-only files on Windows: not implemented yet (protected DACL, spec §6.2).
//! `state_dir` returns `None`, so grant persistence is off unless `MCP_GRANTS_FILE`
//! names a path, and then every write fails loudly instead of storing a secret
//! under an inherited ACL.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const UNSUPPORTED: &str = "owner-only files are not implemented on Windows yet";

pub(crate) fn state_dir() -> Option<PathBuf> {
    None
}

pub(crate) fn create_dir_all(_: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, UNSUPPORTED))
}

pub(crate) fn create_new(_: &Path) -> io::Result<fs::File> {
    Err(io::Error::new(io::ErrorKind::Unsupported, UNSUPPORTED))
}

pub(crate) fn narrow(_: &Path, _: &fs::Metadata) -> Result<(), String> {
    Err(UNSUPPORTED.to_string())
}
