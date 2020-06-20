use crate::arg_parser::ParsedArgs;
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
    CommandInfo {
      name: "repl",
      description: "Starts an interactive REPL session",
      args: None,
      flags: Some(vec![
        Flag::with_names("help", "h").description("Print help text")
      ]),
    }
  }

  fn from_inputs(args: &'a mut ParsedArgs) -> Self {
    ReplCommand { args }
  }

  fn execute(self) -> Result<(), CommandError> {
    self.args.check_valid()?;

    println!("{} repl (version {})\n", BINARY_NAME, VERSION);

    println!("Use Ctrl-D or type '.exit' to quit.");
    println!("Type '.help' for more.");

    let mut repl = repl::Repl::new(PathBuf::from_str(".pluma").ok());

    repl.start();

    Ok(())
  }
}
