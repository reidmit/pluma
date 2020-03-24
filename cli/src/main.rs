#![allow(dead_code)]

use pluma_compiler::compiler::Compiler;
use pluma_compiler::diagnostics::Diagnostic;
use pluma_compiler::VERSION;
use std::fmt;
use std::path::PathBuf;
use std::process::exit;

use crate::options::Command;

mod colors;
mod errors;
mod options;
mod usage;
mod utils;

fn main() {
  match options::parse_options() {
    Ok(Command::Help) => {
      println!("{}", usage::main_usage());
      exit(0);
    }

    Ok(Command::Version) => {
      println!("version {}", VERSION);
      exit(0);
    }

    Ok(Command::BuildHelp) => {
      println!("{}", usage::build_usage());
      exit(0);
    }

    Ok(Command::Build { root_dir, .. }) => {
      let mut compiler = match Compiler::from_dir(root_dir) {
        Ok(c) => c,
        Err(e) => {
          println!("{:#?}", e);
          exit(1);
        }
      };

      match compiler.run() {
        Ok(_) => {
          println!("Compilation succeeded!");
          exit(0);
        }

        Err(diagnostics) => {
          print_diagnostics(&compiler, diagnostics);
          exit(1);
        }
      }
    }

    Ok(_) => {
      unimplemented!();
    }

    Err(err) => {
      print_error(err);
      exit(1);
    }
  }
}

fn print_error<T: fmt::Display>(msg: T) {
  eprintln!("{} {}", colors::bold_red("Error:"), msg);
}

fn print_diagnostics(compiler: &Compiler, diagnostics: Vec<Diagnostic>) {
  let mut first = true;

  for diagnostic in diagnostics {
    if !first {
      eprintln!("")
    }

    eprintln!("{} {}", colors::bold_red("Error:"), diagnostic.message);

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
      let module_name = diagnostic.module_name.unwrap();

      let module = compiler.modules.get(&module_name).unwrap();

      let mut col_index = 0;

      if let Some(bytes) = &module.bytes {
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
          if frame_end >= bytes.len() - 1 {
            break;
          }

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
          colors::bold_red(">"),
          colors::bold_dim(format!("{}|", line).as_str()),
          frame
        );

        let prefix_width = 4 + line.to_string().len();
        let up_arrows = "^".repeat(end - start).to_string();

        eprintln!(
          "{}{}",
          " ".repeat(prefix_width + col_index),
          colors::bold_red(&up_arrows)
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
    }

    first = false;
  }
}
