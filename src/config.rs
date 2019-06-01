use crate::fs;

pub struct CompilerConfig {
  pub entry_path: String,
  pub preserve_comments: bool,
}

impl CompilerConfig {
  pub fn new(given_entry_path: Option<String>) -> Result<CompilerConfig, String> {
    return fs::find_entry_file(given_entry_path).map(|entry_path| CompilerConfig {
      entry_path,
      preserve_comments: false,
    });
  }
}
