use crate::arg_parser::ParsedArgs;
use crate::command::Command;
use crate::command_error::CommandError;
use crate::command_info::*;
use pluma_constants::VERSION;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct VersionCommand<'a> {
  args: &'a mut ParsedArgs,
}

impl<'a> Command<'a> for VersionCommand<'a> {
  fn info() -> CommandInfo {
    CommandInfo::new("version", "Prints compiler version and related information").with_help()
  }


  fn execute(_args: &ParsedArgs) -> Result<(), CommandError> {
    println!("pluma version {}", VERSION);

    Ok(())
  }
}
