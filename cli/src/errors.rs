use std::fmt;

#[derive(Debug)]
pub enum UsageError {
  NoCommand,
  UnknownCommand(String),
  MissingEntryPath,
  InvalidEntryPath(String),
  EntryDirDoesNotContainEntryFile(String),
}

impl fmt::Display for UsageError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    write!(f, "{:#?}", self)
  }
}