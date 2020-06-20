use crate::arg_parser::ParsedArgs;
use crate::colors;
use crate::command::Command;
use pluma_constants::*;
use pluma_repl::repl;
use std::path::PathBuf;
use std::str::FromStr;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ReplCommand {}

impl Command for ReplCommand {
  fn help_text() -> String {
    format!(
      "{binary_name} repl

Starts an interactive REPL session

{usage_header}
  {cmd_prefix} {binary_name} repl

{options_header}
  -h, --help    Print this help text",
      usage_header = colors::bold("Usage:"),
      binary_name = BINARY_NAME,
      options_header = colors::bold("Options:"),
      cmd_prefix = colors::dim("$"),
    )
  }

  fn from_inputs(_args: ParsedArgs) -> Self {
    ReplCommand {}
  }

  fn execute(self) {
    println!("{} repl (version {})\n", BINARY_NAME, VERSION);

    println!("Use Ctrl-D or type '.exit' to quit.");
    println!("Type '.help' for more.");

    let mut repl = repl::Repl::new(PathBuf::from_str(".pluma").ok());

    repl.start();
  }
}
