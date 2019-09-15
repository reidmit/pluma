use std::fs;
use std::path::PathBuf;
use crate::errors::FileError;
use crate::FILE_EXTENSION;

pub fn read_file_contents(abs_file_path: &String) -> Result<Vec<u8>, FileError> {
  match fs::read(abs_file_path) {
    Ok(bytes) => Ok(bytes),
    Err(_) => Err(FileError::FailedToReadFile(abs_file_path.to_string())),
  }
}

pub fn to_absolute_path(root_dir: &String, module_name: &String) -> String {
  let mut path = PathBuf::new();

  path.push(root_dir);
  let parts = module_name.split("/");
  for part in parts { path.push(part) }
  path.set_extension(FILE_EXTENSION);

  path.as_path()
    .canonicalize()
    .expect("Failed to canonicalize")
    .to_str()
    .unwrap()
    .to_owned()
}