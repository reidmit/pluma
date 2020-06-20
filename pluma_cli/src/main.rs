use crate::command::Command;
use crate::commands::*;
use std::process::exit;

mod arg_parser;
mod colors;
mod command;
mod command_error;
mod command_info;
mod commands;
mod errors;

fn main() {
  if let Err(err) = run() {
    errors::print_command_error(err);
    exit(1);
  }
}

fn run() -> Result<(), command_error::CommandError> {
  let args = std::env::args().skip(1).collect();
  let mut parsed_args = arg_parser::parse_args(args);

  match &parsed_args.subcommand()[..] {
    "build" => {
      if parsed_args.is_help_requested() {
        BuildCommand::print_help();
      } else {
        BuildCommand::from_inputs(&mut parsed_args).execute()?;
      }
    }

    "check" => {
      if parsed_args.is_help_requested() {
        CheckCommand::print_help();
      } else {
        CheckCommand::from_inputs(&mut parsed_args).execute()?;
      }
    }

    "run" => {
      if parsed_args.is_help_requested() {
        RunCommand::print_help();
      } else {
        RunCommand::from_inputs(&mut parsed_args).execute()?;
      }
    }

    "repl" => {
      if parsed_args.is_help_requested() {
        ReplCommand::print_help();
      } else {
        ReplCommand::from_inputs(&mut parsed_args).execute()?;
      }
    }

    "version" => {
      if parsed_args.is_help_requested() {
        VersionCommand::print_help();
      } else {
        VersionCommand::from_inputs(&mut parsed_args).execute()?;
      }
    }

    "help" => {
      if parsed_args.is_help_requested() {
        HelpCommand::print_help();
      } else {
        HelpCommand::from_inputs(&mut parsed_args).execute()?;
      }
    }

    "" => {
      errors::print_usage_error(format!("No command given."));
      exit(1);
    }

    unknown => {
      errors::print_usage_error(format!("Command '{}' is not recognized.", unknown));
      exit(1);
    }
  }

  Ok(())
}
