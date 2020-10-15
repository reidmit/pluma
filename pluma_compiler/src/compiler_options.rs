#[cfg_attr(debug_assertions, derive(Debug))]
pub enum CompilerMode {
  Debug,
  Release,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct CompilerOptions {
  pub entry_path: String,
  pub mode: CompilerMode,
  pub output_path: Option<String>,
}
