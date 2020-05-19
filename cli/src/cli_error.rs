use std::fmt;

#[derive(Debug)]
pub enum CliError {
  NoCommand,
  UnknownCommand(String),
  MissingEntryPath,
}

impl fmt::Display for CliError {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use CliError::*;

    match self {
      MissingEntryPath => write!(f, "No entry path provided."),
      UnknownCommand(name) => write!(f, "Command '{}' is not recognized.", name),
      other => write!(f, "{:#?}", other),
    }
  }
}
