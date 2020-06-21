use crate::arg_parser::ParsedArgs;
use crate::colors;
use crate::command::Command;
use crate::command_error::CommandError;
use crate::command_info::*;
use pluma_constants::*;
use pluma_repl::repl;
use std::path::PathBuf;
use std::str::FromStr;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ReplCommand<'a> {
  args: &'a mut ParsedArgs,
}

impl<'a> Command<'a> for ReplCommand<'a> {
  fn info() -> CommandInfo {
    CommandInfo::new("repl", "Starts an interactive REPL session").with_help()
  }

  fn execute(_args: &ParsedArgs) -> Result<(), CommandError> {
    println!(
      "{} {} - version {}",
      colors::bold(BINARY_NAME),
      colors::bold("repl"),
      VERSION
    );
    println!("Use Ctrl-D or type '.exit' to quit.");
    println!("Type '.help' for more.");

    let mut repl = repl::Repl::new(PathBuf::from_str(".pluma").ok());

    repl.start();

    Ok(())
  }
}
