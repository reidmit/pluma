use pluma_compiler::VERSION;
use crate::colors;

pub fn main_usage() -> String {
  format!(
    "{bold_name} - version {version}

Compiler and tools for the Pluma language

{usage_header}
  {cmd_prefix} {cli_name} <command> [...options]

{commands_header}
  build     Compile a given module
  run       Compile and run a given module
  help      Print this help text
  version   Print version

For help with an individual command, try:
  {cmd_prefix} {cli_name} <command> -h
",
    bold_name = colors::bold("pluma"),
    cli_name = "pluma",
    version = VERSION,
    usage_header = colors::bold("Usage:"),
    commands_header = colors::bold("Commands:"),
    cmd_prefix = colors::dim("$"),
  )
}

pub fn build_usage() -> String {
  format!(
    "{bold_name} {cmd_name} - version {version}

Compile a Pluma module.

{usage_header}
  {cmd_prefix} {cli_name} build [module] [...options]

{args_header}
  module   Path to entry file or directory (default: '.')

{options_header}
  -o, --out    Path to output file (default: './out.plc')
  -h, --help   Print this help text and exit
",
    bold_name = colors::bold("pluma"),
    cmd_name = colors::bold("build"),
    cli_name = "pluma",
    version = VERSION,
    usage_header = colors::bold("Usage:"),
    args_header = colors::bold("Arguments:"),
    options_header = colors::bold("Options:"),
    cmd_prefix = colors::dim("$"),
  )
}