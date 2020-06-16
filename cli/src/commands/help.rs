use crate::colors;
use crate::commands;
use pluma_compiler::{BINARY_NAME, VERSION};

pub fn description() -> String {
  format!("{}", "Compiles a module into an executable")
}

pub fn print_help() {
  println!(
    "{description}

{usage_header}
    {cmd_prefix} {binary_name} help [<command>]

{arguments_header}
    <command>    Name of command to print help for

{options_header}
    -h, --help    Print this help text",
    description = description(),
    usage_header = colors::bold("Usage:"),
    binary_name = BINARY_NAME,
    arguments_header = colors::bold("Arguments:"),
    options_header = colors::bold("Options:"),
    cmd_prefix = colors::dim("$"),
  )
}

pub fn execute() {
  println!(
    "{binary_name_bold} - version {version}

Compiler & tools for the Pluma language

{usage_header}
    {cmd_prefix} {binary_name} <command> [options...]

{commands_header}
    build     {build_description}
    check     {check_description}
    repl      {repl_description}
    run       {run_description}
    version   {version_description}
    help      {help_description}

For help with an individual command, try:
    {cmd_prefix} {binary_name} help <command>",
    binary_name_bold = format!("{}", colors::bold(BINARY_NAME)),
    binary_name = BINARY_NAME,
    version = VERSION,
    usage_header = colors::bold("Usage:"),
    commands_header = colors::bold("Commands:"),
    build_description = commands::build::description(),
    check_description = commands::check::description(),
    repl_description = commands::repl::description(),
    run_description = commands::run::description(),
    version_description = commands::version::description(),
    help_description = commands::help::description(),
    cmd_prefix = colors::dim("$"),
  )
}
