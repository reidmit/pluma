use crate::command::Command;
use crate::commands::*;

mod arg_parser;
mod colors;
mod command;
mod commands;
mod errors;

fn main() {
  let args = std::env::args().skip(1).collect();
  let parsed_args = arg_parser::parse_args(args);

  match &parsed_args.subcommand()[..] {
    "build" => {
      if parsed_args.is_help_requested() {
        BuildCommand::print_help();
      } else {
        BuildCommand::from_inputs(parsed_args).execute();
      }
    }

    "check" => {
      if parsed_args.is_help_requested() {
        CheckCommand::print_help();
      } else {
        CheckCommand::from_inputs(parsed_args).execute();
      }
    }

    "run" => {
      if parsed_args.is_help_requested() {
        RunCommand::print_help();
      } else {
        RunCommand::from_inputs(parsed_args).execute();
      }
    }

    "repl" => {
      if parsed_args.is_help_requested() {
        ReplCommand::print_help();
      } else {
        ReplCommand::from_inputs(parsed_args).execute();
      }
    }

    "version" => {
      if parsed_args.is_help_requested() {
        VersionCommand::print_help();
      } else {
        VersionCommand::from_inputs(parsed_args).execute();
      }
    }

    "help" => {
      if parsed_args.is_help_requested() {
        HelpCommand::print_help();
      } else {
        match parsed_args.get_positional_arg(0) {
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

          _ => HelpCommand::from_inputs(parsed_args).execute(),
        }
      }
    }

    "" => {
      errors::print_usage_error(format!("No command given."));
      std::process::exit(1);
    }

    unknown => {
      errors::print_usage_error(format!("Command '{}' is not recognized.", unknown));
      std::process::exit(1);
    }
  }
}
