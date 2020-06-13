use crate::colors;
use pluma_compiler::{BINARY_NAME, VERSION};

pub fn main_help_template() -> String {
  format!(
    "{bold_name} - version {version}

{{about}}

{usage_header}
    {cmd_prefix} {{bin}} <command>

{commands_header}
{{subcommands}}

{flags_header}
{{flags}}

For help with an individual command, try:
    {cmd_prefix} {{bin}} help <command>",
    bold_name = colors::bold(BINARY_NAME),
    version = VERSION,
    usage_header = colors::bold("Usage:"),
    commands_header = colors::bold("Commands:"),
    flags_header = colors::bold("Flags:"),
    cmd_prefix = colors::dim("$"),
  )
}

pub fn command_help_template() -> String {
  format!(
    "{{about}}

{usage_header}
    {cmd_prefix} {{usage}}

{arguments_header}
{{positionals}}

{options_header}
{{options}}

{flags_header}
{{flags}}",
    usage_header = colors::bold("Usage:"),
    arguments_header = colors::bold("Arguments:"),
    options_header = colors::bold("Options:"),
    flags_header = colors::bold("Flags:"),
    cmd_prefix = colors::dim("$"),
  )
}

pub fn command_help_template_no_options() -> String {
  format!(
    "{{about}}

{usage_header}
    {cmd_prefix} {{usage}}

{arguments_header}
{{positionals}}

{flags_header}
{{flags}}",
    usage_header = colors::bold("Usage:"),
    arguments_header = colors::bold("Arguments:"),
    flags_header = colors::bold("Flags:"),
    cmd_prefix = colors::dim("$"),
  )
}
