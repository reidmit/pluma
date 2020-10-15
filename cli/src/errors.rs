use crate::colors;
use crate::command_error::CommandError;
use constants::*;
use diagnostics::*;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub fn print_command_error(err: CommandError) {
  eprintln!(
    "{prefix} {err}",
    prefix = colors::bold_red("Error:"),
    err = err
  )
}

pub fn print_usage_error(message: String) {
  eprintln!(
    "{prefix} {message}\n\nFor help and a list of available commands, try:\n  {binary_name} help",
    prefix = colors::bold_red("Error:"),
    message = message,
    binary_name = BINARY_NAME,
  )
}

pub fn print_diagnostics(diagnostics: Vec<Diagnostic>) {
  let mut first = true;
  let mut module_bytes = HashMap::<PathBuf, Vec<u8>>::new();

  for diagnostic in diagnostics {
    if !first {
      eprintln!("")
    }

    let is_error = diagnostic.is_error();

    eprintln!(
      "{} {}",
      if is_error {
        colors::bold_red("Error:")
      } else {
        colors::bold_yellow("Warning:")
      },
      diagnostic.message
    );

    if diagnostic.module_path.is_none() {
      continue;
    }

    let mut module_path = diagnostic.module_path.unwrap();
    let cwd = std::env::current_dir().unwrap_or(PathBuf::from(""));
    if module_path.starts_with(cwd) {
      module_path = module_path
        .strip_prefix(std::env::current_dir().unwrap())
        .unwrap()
        .to_path_buf();
    }

    if let Some((start, end)) = diagnostic.pos {
      let mut col_index = 0;

      let bytes = match module_bytes.get(&module_path) {
        Some(bytes) => bytes,
        None => match fs::read(&module_path) {
          Ok(bytes) => {
            module_bytes.insert(module_path.clone(), bytes);
            module_bytes.get(&module_path).unwrap()
          }
          _ => unreachable!(),
        },
      };

      let mut frame_start = start;
      let mut frame_end = end;

      while frame_start > 0 {
        if let Some(b'\n') = bytes.get(frame_start - 1) {
          break;
        }

        col_index += 1;
        frame_start -= 1
      }

      while let Some(byte) = bytes.get(frame_end) {
        match byte {
          b'\n' => break,
          _ => frame_end += 1,
        }
      }

      let frame = String::from_utf8(bytes[frame_start..frame_end].to_vec())
        .unwrap()
        .replace("\n", " ");

      let mut line = 1;

      frame_start = start;
      while frame_start > 0 {
        if let Some(b'\n') = bytes.get(frame_start - 1) {
          line += 1;
        }

        frame_start -= 1;
      }

      eprintln!(
        "\n{} {} {}",
        if is_error {
          colors::bold_red(">")
        } else {
          colors::bold_yellow(">")
        },
        colors::bold_dim(format!("{}|", line).as_str()),
        frame
      );

      let prefix_width = 4 + line.to_string().len();
      let up_arrows = "^".repeat(end - start).to_string();

      eprintln!(
        "{}{}",
        " ".repeat(prefix_width + col_index),
        if is_error {
          colors::bold_red(&up_arrows)
        } else {
          colors::bold_yellow(&up_arrows)
        }
      );

      eprintln!(
        "{}",
        colors::dim(
          format!(
            "{}:{}:{}",
            module_path.to_str().unwrap(),
            line,
            col_index + 1
          )
          .as_str()
        )
      );
    }

    first = false;
  }
}
