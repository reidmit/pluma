use crate::arg_parser::ParsedArgs;
use crate::command_error::CommandError;
use crate::command_info::CommandInfo;

pub trait Command<'a> {
  fn info() -> CommandInfo;

  fn execute(args: &ParsedArgs) -> Result<(), CommandError>;

  fn print_help() {
    println!("{}", Self::info());
  }

  fn description() -> &'static str {
    Self::info().description
  }
}
