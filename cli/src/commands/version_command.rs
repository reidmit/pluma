use crate::arg_parser::ParsedArgs;
use crate::command::Command;
use crate::command_error::CommandError;
use crate::command_info::*;
use constants::VERSION;

pub struct VersionCommand {}

impl Command for VersionCommand {
  fn info() -> CommandInfo {
    CommandInfo::new("version", "Prints compiler version and related information").with_help()
  }

  fn execute(_args: &ParsedArgs) -> Result<(), CommandError> {
    println!("pluma version {}", VERSION);

    Ok(())
  }
}
