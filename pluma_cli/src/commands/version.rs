use crate::colors;
use pluma_constants::{BINARY_NAME, VERSION};

pub fn description() -> String {
  format!("{}", "Prints compiler version and information")
}

pub fn print_help() {
  println!(
    "{description}

{usage_header}
  {cmd_prefix} {binary_name} version

{options_header}
  -h, --help    Print this help text",
    description = description(),
    usage_header = colors::bold("Usage:"),
    binary_name = BINARY_NAME,
    options_header = colors::bold("Options:"),
    cmd_prefix = colors::dim("$"),
  )
}

pub fn execute() {
  println!("pluma version {}", VERSION);
}
