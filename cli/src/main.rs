use crate::command::Command;
use crate::command_error::*;
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
  let (subcommand, is_help_requested, args) = arg_parser::find_subcommand();

  if subcommand.is_none() {
    let mut parsed_args = arg_parser::parse_args_for_command(args, HelpCommand::info())?;
    HelpCommand::execute(&mut parsed_args)?;
    return Ok(());
  }

  match &subcommand.unwrap()[..] {
    "check" => {
      if is_help_requested {
        CheckCommand::print_help();
      } else {
        let mut parsed_args = arg_parser::parse_args_for_command(args, CheckCommand::info())?;
        CheckCommand::execute(&mut parsed_args)?;
      }
    }

    "version" => {
      if is_help_requested {
        VersionCommand::print_help();
      } else {
        let mut parsed_args = arg_parser::parse_args_for_command(args, VersionCommand::info())?;
        VersionCommand::execute(&mut parsed_args)?;
      }
    }

    "help" => {
      if is_help_requested {
        HelpCommand::print_help();
      } else {
        let mut parsed_args = arg_parser::parse_args_for_command(args, HelpCommand::info())?;
        HelpCommand::execute(&mut parsed_args)?;
      }
    }

    unknown => {
      return Err(CommandError {
        command: "".to_owned(),
        kind: CommandErrorKind::UnexpectedCommand(unknown.to_owned()),
      });
    }
  }

  Ok(())
}
