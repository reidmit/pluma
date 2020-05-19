use crate::{DEFAULT_ENTRY_MODULE_NAME, FILE_EXTENSION};
use std::fmt;

#[derive(Debug, Clone)]
pub struct UsageError {
  pub kind: UsageErrorKind,
}

#[derive(Debug, Clone)]
pub enum UsageErrorKind {
  InvalidEntryPath(String),
  EntryDirDoesNotContainEntryFile(String),
}

impl fmt::Display for UsageError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use UsageErrorKind::*;

    match &self.kind {
      EntryDirDoesNotContainEntryFile(dir) => write!(
        f,
        "Directory '{}' does not contain a valid entry module ('{}.{}').",
        dir, DEFAULT_ENTRY_MODULE_NAME, FILE_EXTENSION
      ),
      InvalidEntryPath(path) => write!(f, "Path '{}' is invalid.", path),
    }
  }
}
