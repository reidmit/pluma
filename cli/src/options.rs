use std::env;
use std::path::Path;
use crate::errors::UsageError;
use pluma_compiler::DEFAULT_ENTRY_FILE;

pub enum Command {
  Build {
    root_dir: String,
    entry_path: String,
  },
  BuildHelp,
  Run {
    root_dir: String,
    entry_path: String,
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
    None => return Err(UsageError::NoCommand)
  };

  match command_name.as_str() {
    "help" => {
      Ok(Command::Help)
    },

    "version" => {
      Ok(Command::Version)
    },

    "build" => {
      if show_help() {
        return Ok(Command::BuildHelp)
      }

      let entry_path = match env::args().nth(2) {
        Some(file) => file,
        None => return Err(UsageError::MissingEntryPath)
      };

      let (root_dir, entry_path) = get_paths(entry_path)?;

      Ok(Command::Build {
        root_dir,
        entry_path,
      })
    },

    "run" => {
      if show_help() {
        return Ok(Command::RunHelp)
      }

      let entry_path = match env::args().nth(2) {
        Some(file) => file,
        None => return Err(UsageError::MissingEntryPath)
      };

      let (root_dir, entry_path) = get_paths(entry_path)?;

      Ok(Command::Run {
        root_dir,
        entry_path,
      })
    },

    other => Err(UsageError::UnknownCommand(other.to_owned()))
  }
}

fn show_help() -> bool {
  for arg in env::args() {
    if arg == "-h" || arg == "--help" {
      return true
    }
  }

  return false
}

fn get_paths(entry_path: String) -> Result<(String, String), UsageError> {
  let joined_path = Path::new(&env::current_dir().unwrap()).join(entry_path);

  match joined_path.canonicalize() {
    Ok(abs_path) => {
      if abs_path.is_dir() {
        return match abs_path.join(DEFAULT_ENTRY_FILE).canonicalize() {
          Ok(abs_file_path) => Ok((
            abs_path.to_str().unwrap().to_owned(),
            abs_file_path.to_str().unwrap().to_owned()
          )),
          Err(_) => Err(UsageError::EntryDirDoesNotContainEntryFile(
            joined_path.to_str().unwrap().to_owned()
          ))
        }
      }

      Ok((
        abs_path.parent().unwrap().to_str().unwrap().to_owned(),
        abs_path.to_str().unwrap().to_owned()
      ))
    },

    Err(_) => Err(UsageError::InvalidEntryPath(
      joined_path.to_str().unwrap().to_owned()
    ))
  }
}