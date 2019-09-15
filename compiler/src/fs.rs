use std::fs;
use std::path::{Path, PathBuf};
use crate::errors::FileError;
use crate::FILE_EXTENSION;

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

pub fn to_absolute_path(root_dir: &String, path: &String) -> String {
  Path::new(root_dir)
    .join(path)
    .canonicalize()
    .expect("Failed to canonicalize")
    .to_str()
    .unwrap()
    .to_owned()
}