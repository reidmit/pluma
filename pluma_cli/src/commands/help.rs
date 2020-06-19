use crate::arg_parser::ParsedArgs;
use crate::colors;
use crate::command::Command;
use pluma_constants::{BINARY_NAME, VERSION};

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct HelpCommand {}

impl Command for HelpCommand {
  fn help_text() -> String {
    format!(
      "{binary_name} help

Prints help text for the Pluma CLI or a particular command

{usage_header}
  {cmd_prefix} {binary_name} help [<command>]

{arguments_header}
  <command>    Name of command to print help for

{options_header}
  -h, --help    Print this help text",
      usage_header = colors::bold("Usage:"),
      binary_name = BINARY_NAME,
      arguments_header = colors::bold("Arguments:"),
      options_header = colors::bold("Options:"),
      cmd_prefix = colors::dim("$"),
    )
  }

  fn from_inputs(_args: ParsedArgs) -> Self {
    HelpCommand {}
  }

  fn execute(self) {
    println!(
      "{binary_name_bold} - version {version}

Compiler & tools for the Pluma language

{usage_header}
  {cmd_prefix} {binary_name} <command> [options...]

{commands_header}
  build     TODO
  check     TODO
  repl      TODO
  run       TODO
  version   TODO
  help      TODO

For help with an individual command, try:
  {cmd_prefix} {binary_name} help <command>",
      binary_name_bold = format!("{}", colors::bold(BINARY_NAME)),
      binary_name = BINARY_NAME,
      version = VERSION,
      usage_header = colors::bold("Usage:"),
      commands_header = colors::bold("Commands:"),
      cmd_prefix = colors::dim("$"),
    )
  }
}
