use std::fs;
use std::path::Path;
use crate::errors::{ConfigurationError, FileError};

const DEFAULT_ENTRY_FILE: &str = "main.pa";

pub fn find_entry_file(given_entry_file: Option<String>) -> Result<String, ConfigurationError> {
  let f = given_entry_file.unwrap_or_else(|| DEFAULT_ENTRY_FILE.to_string());
  let path = Path::new(&f);

  if path.is_file() {
    return Ok(path.canonicalize().unwrap().to_str().unwrap().to_owned());
  } else {
    let merged_path = path.join(DEFAULT_ENTRY_FILE);

    return match merged_path.canonicalize() {
      Ok(path) => Ok(path.to_str().unwrap().to_owned()),
      Err(_) => Err(ConfigurationError::EntryPathDoesNotExist(merged_path.to_str().unwrap().to_owned())),
    };
  }
}

pub fn read_file_contents(abs_file_path: &String) -> Result<Vec<u8>, FileError> {
  match fs::read(abs_file_path) {
    Ok(bytes) => Ok(bytes),
    Err(_) => Err(FileError::FailedToReadFile(abs_file_path.to_string())),
  }
}
