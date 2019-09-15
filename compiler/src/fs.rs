use std::fs;
use std::path::{Path, PathBuf};
use crate::errors::{ConfigurationError, FileError};

const DEFAULT_ENTRY_FILE: &str = "main.pa";
const FILE_EXTENSION: &str = "pa";

pub fn find_root_dir_and_entry_file(given_entry_file: Option<String>) -> Result<(String, String), ConfigurationError> {
  let f = given_entry_file.unwrap_or_else(|| DEFAULT_ENTRY_FILE.to_string());
  let path = Path::new(&f);

  if path.is_file() {
    let root_dir = path.parent().unwrap().to_str().unwrap().to_owned();
    let entry_file = path.canonicalize().unwrap().to_str().unwrap().to_owned();

    return Ok((root_dir, entry_file));
  } else {
    let merged_path = path.join(DEFAULT_ENTRY_FILE);
    let root_dir = path.to_str().unwrap().to_owned();

    return match merged_path.canonicalize() {
      Ok(path) => {
        let entry_file = path.to_str().unwrap().to_owned();

        Ok((root_dir, entry_file))
      },
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

pub fn get_full_path_from_import(root_dir: &String, import_path: &String) -> String {
  let parts = import_path.split("/");

  let mut path = PathBuf::new();
  path.push(root_dir);
  for part in parts { path.push(part) }
  path.set_extension(FILE_EXTENSION);

  path.to_str().unwrap().to_owned()
}

pub fn to_absolute_path(path: &String) -> String {
  Path::new(path).canonicalize().unwrap().to_str().unwrap().to_owned()
}