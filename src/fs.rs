use crate::constants;
use std::fs;
use std::path::Path;

pub fn find_entry_file(file_or_dir: Option<String>) -> Result<String, String> {
  let f = file_or_dir.unwrap_or_else(|| constants::DEFAULT_ENTRY_FILE.to_string());
  let path = Path::new(&f);

  if path.is_file() {
    return Ok(path.canonicalize().unwrap().to_str().unwrap().to_owned());
  } else {
    let merged_path = path.join(constants::DEFAULT_ENTRY_FILE);

    return match merged_path.canonicalize() {
      Ok(path) => Ok(path.to_str().unwrap().to_owned()),
      Err(_) => Err(format!(
        "Failed to find file: {}",
        merged_path.to_str().unwrap().to_owned()
      )),
    };
  }
}

pub fn read_file_contents(abs_file_path: &String) -> Result<Vec<u8>, String> {
  return match fs::read(abs_file_path) {
    Ok(bytes) => Ok(bytes),
    Err(_) => Err(format!("Failed to read file: {}", abs_file_path)),
  };
}
