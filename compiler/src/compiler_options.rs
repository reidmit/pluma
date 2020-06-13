#[derive(Debug)]
pub enum CompilerMode {
  Debug,
  Release,
}

pub struct CompilerOptions {
  pub entry_path: String,
  pub mode: CompilerMode,
  pub output_path: Option<String>,
  pub execute_after_compilation: bool,
}
