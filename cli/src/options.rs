use crate::errors::UsageError;
use std::env;
use std::path::PathBuf;

pub enum Command {
  Build {
    entry_path: String,
  },
  BuildHelp,
  Run {
    root_dir: PathBuf,
    entry_module_name: String,
  },
  RunHelp,
  Help,
  Version,
}

pub fn parse_options() -> Result<Command, UsageError> {
  if env::args().len() < 2 {
    return Ok(Command::Help);
  }

  let command_name = match env::args().nth(1) {
    Some(name) => name,
    None => return Err(UsageError::NoCommand),
  };

  match command_name.as_str() {
    "help" => Ok(Command::Help),

    "version" => Ok(Command::Version),

    "build" => {
      if show_help() {
        return Ok(Command::BuildHelp);
      }

      let entry_path = match env::args().nth(2) {
        Some(file) => file,
        None => return Err(UsageError::MissingEntryPath),
      };

      Ok(Command::Build { entry_path })
    }

    "run" => todo!(),

    other => Err(UsageError::UnknownCommand(other.to_owned())),
  }
}

fn show_help() -> bool {
  for arg in env::args_os() {
    if arg == "-h" || arg == "--help" {
      return true;
    }
  }

  return false;
}
