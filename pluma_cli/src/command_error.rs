use pluma_constants::*;
use std::fmt;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct CommandError {
  pub command: String,
  pub kind: CommandErrorKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum CommandErrorKind {
  UnexpectedArgument(String),
  UnexpectedFlag(String),
}

impl fmt::Display for CommandError {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match &self.kind {
      CommandErrorKind::UnexpectedArgument(arg) => write!(f, "Unexpected argument '{}'.", arg,)?,

      CommandErrorKind::UnexpectedFlag(flag) => write!(f, "Unknown flag '{}'.", flag)?,
    }

    write!(
      f,
      "\n\nFor help with this command, try:\n  {} {} -h",
      BINARY_NAME, self.command
    )?;

    Ok(())
  }
}
