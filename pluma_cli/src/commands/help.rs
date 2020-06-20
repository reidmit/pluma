use crate::arg_parser::ParsedArgs;
use crate::colors;
use crate::command::Command;
use crate::command_error::CommandError;
use crate::command_info::*;
use crate::commands::*;
use crate::errors;
use pluma_constants::{BINARY_NAME, VERSION};

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct HelpCommand<'a> {
  subcommand: Option<String>,
  args: &'a mut ParsedArgs,
}

impl<'a> Command<'a> for HelpCommand<'a> {
  fn info() -> CommandInfo {
    CommandInfo::new(
      "help",
      "Prints help text for the compiler or a particular command",
    )
    .with_help()
  }

  fn from_inputs(args: &'a mut ParsedArgs) -> Self {
    HelpCommand {
      subcommand: args.get_positional_arg(0),
      args,
    }
  }

  fn execute(self) -> Result<(), CommandError> {
    self.args.check_valid()?;

    match self.subcommand {
      Some(val) => match &val[..] {
        "build" => BuildCommand::print_help(),
        "check" => CheckCommand::print_help(),
        "run" => RunCommand::print_help(),
        "help" => HelpCommand::print_help(),
        "repl" => ReplCommand::print_help(),
        "version" => VersionCommand::print_help(),
        unknown => {
          errors::print_usage_error(format!(
            "Cannot retrieve help for unrecognized command '{}'.",
            unknown
          ));

          std::process::exit(1);
        }
      },

      _ => {
        println!(
          "{binary_name_bold} - version {version}

Compiler & tools for the Pluma language

{usage_header}
  {binary_name} <command> [options]

{commands_header}",
          binary_name_bold = format!("{}", colors::bold(BINARY_NAME)),
          binary_name = BINARY_NAME,
          version = VERSION,
          usage_header = colors::bold("Usage:"),
          commands_header = colors::bold("Commands:"),
        );

        let cmd_info: Vec<CommandInfo> = vec![
          BuildCommand::info(),
          CheckCommand::info(),
          ReplCommand::info(),
          RunCommand::info(),
          VersionCommand::info(),
          HelpCommand::info(),
        ];

        let mut max_cmd_length = 0;
        for info in &cmd_info {
          max_cmd_length = std::cmp::max(max_cmd_length, info.name.len());
        }

        for info in cmd_info {
          println!(
            "  {:width$}   {}",
            info.name,
            info.description,
            width = max_cmd_length
          );
        }

        println!(
          "\nFor help with an individual command, try:
  {binary_name} <command> -h",
          binary_name = BINARY_NAME,
        )
      }
    }

    Ok(())
  }
}
