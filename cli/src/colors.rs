use std::env;
use atty::Stream;

fn hide_colors() -> bool {
  if atty::is(Stream::Stdout) {
    return false
  }

  match env::var("NO_COLOR") {
    Ok(value) => value == "1",
    _ => false,
  }
}

pub fn bold(text: &str) -> String {
  if hide_colors() {
    return format!("{}", text);
  }

  return format!("\x1b[1m{}\x1b[0m", text);
}

pub fn dim(text: &str) -> String {
  if hide_colors() {
    return format!("{}", text);
  }

  return format!("\x1b[2m{}\x1b[0m", text);
}

pub fn red(text: &str) -> String {
  if hide_colors() {
    return format!("{}", text);
  }

  return format!("\x1b[31m{}\x1b[0m", text);
}

pub fn bold_red(text: &str) -> String {
  if hide_colors() {
    return format!("{}", text);
  }

  return format!("\x1b[1m\x1b[31m{}\x1b[0m", text);
}
