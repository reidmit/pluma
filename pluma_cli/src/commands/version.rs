use crate::arg_parser::ParsedArgs;
use crate::colors;
use crate::command::Command;
use pluma_constants::{BINARY_NAME, VERSION};

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct VersionCommand {}

impl Command for VersionCommand {
  fn help_text() -> String {
    format!(
      "{binary_name} version

Prints compiler version and related information

{usage_header}
  {cmd_prefix} {binary_name} version

{options_header}
  -h, --help    Print this help text",
      usage_header = colors::bold("Usage:"),
      binary_name = BINARY_NAME,
      options_header = colors::bold("Options:"),
      cmd_prefix = colors::dim("$"),
    )
  }

  fn from_inputs(_args: ParsedArgs) -> Self {
    VersionCommand {}
  }

  fn execute(self) {
    println!("pluma version {}", VERSION);
  }
}
