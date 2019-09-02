use std::fmt;

#[derive(Debug)]
pub struct UsageError {
  message: String,
}

impl fmt::Display for UsageError {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "{}", self.message)
  }
}

impl UsageError {
  pub fn unknown_command(command_name: String) -> UsageError {
    UsageError {
      message: format!("Unknown command: {}", command_name),
    }
  }
}
