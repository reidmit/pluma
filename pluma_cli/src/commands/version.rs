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

  fn from_inputs(args: &'a mut ParsedArgs) -> Self {
    VersionCommand { args }
  }

  fn execute(self) -> Result<(), CommandError> {
    self.args.check_valid()?;

    println!("pluma version {}", VERSION);

    Ok(())
  }
}
