use constants::*;
use std::fmt;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct CommandError {
	pub command: String,
	pub kind: CommandErrorKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum CommandErrorKind {
	UnexpectedCommand(String),
	UnexpectedArgument(String),
	UnexpectedFlag(String),
	MissingValueForFlag(String),
	DuplicateValueForFlag(String),
	InvalidValueForFlag(String, String),
}

impl fmt::Display for CommandError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match &self.kind {
			CommandErrorKind::UnexpectedCommand(arg) => {
				write!(f, "Command '{}' is not recognized.", arg)?
			}
			CommandErrorKind::UnexpectedArgument(arg) => {
				write!(f, "Unexpected argument '{}'.", arg)?
			}
			CommandErrorKind::UnexpectedFlag(flag) => {
				write!(f, "Flag '{}' is not recognized.", flag)?
			}
			CommandErrorKind::MissingValueForFlag(flag) => {
				write!(f, "Flag '{}' needs a value.", flag)?
			}
			CommandErrorKind::DuplicateValueForFlag(flag) => {
				write!(f, "Flag '{}' can only be specified once.", flag)?
			}
			CommandErrorKind::InvalidValueForFlag(flag, value) => {
				write!(f, "Invalid value '{}' for flag '{}'.", value, flag)?
			}
		}

		if self.command.is_empty() {
			write!(
				f,
				"\n\nFor help and a list of available commands, try:\n  {} help",
				BINARY_NAME
			)?;
		} else {
			write!(
				f,
				"\n\nFor help with this command, try:\n  {} {} -h",
				BINARY_NAME, self.command
			)?;
		}

		Ok(())
	}
}
