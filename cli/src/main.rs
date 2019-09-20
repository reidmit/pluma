use pluma_compiler::compiler::{Compiler, CompilerConfig};
use pluma_compiler::error_formatter::ErrorFormatter;
use pluma_compiler::errors::{PackageCompilationErrorSummary, ModuleCompilationErrorDetail};
use pluma_compiler::VERSION;
use std::fmt;
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
    },

    Ok(Command::Version) => {
      println!("v{}", VERSION);
      exit(0);
    },

    Ok(Command::BuildHelp) => {
      println!("{}", usage::build_usage());
      exit(0);
    },

    Ok(Command::Build { root_dir, entry_module_name }) => {
      let mut compiler = Compiler::new(CompilerConfig {
        root_dir,
        entry_module_name
      });

      match compiler.run() {
        Ok(_) => {
          println!("{:#?}", compiler);
          println!("Compilation succeeded!");
          exit(0);
        }

        Err(e) => {
          let error_formatter = ErrorFormatter::new(&compiler, e);
          print_error_summary(&compiler, error_formatter.get_error_summary());
          exit(1);
        }
      }
    },

    Ok(Command::RunHelp) => {
      unimplemented!();
    },

    Ok(Command::Run { .. }) => {
      unimplemented!();
    },

    Err(err) => {
      print_error(err);
      exit(1);
    }
  }
}

fn print_error<T: fmt::Display>(msg: T) {
  eprintln!("{} {}", colors::bold_red("Error:"), msg);
}

fn print_error_summary(compiler: &Compiler, summary: PackageCompilationErrorSummary) {
  if !summary.package_errors.is_empty() {
    for package_error in summary.package_errors {
      print_error(package_error);
    }

    return
  }

  if !summary.module_errors.is_empty() {
    eprintln!("{} while compiling:",
      colors::bold_red("Error(s)"),
    );
  }

  for (module_name, module_errors) in summary.module_errors {
    for ModuleCompilationErrorDetail { module_path, location, message } in module_errors {
      eprintln!("\n── module: {} {}\n",
        colors::bold(module_name.as_str()),
        "─".repeat(utils::get_terminal_width() - module_name.len() - 12),
      );

      eprintln!("{}", message);

      if let Some((start, end)) = location {
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
              break
            }

            match byte {
              b'\n' => break,
              _ => frame_end += 1
            }
          }

          let frame = String::from_utf8(bytes[frame_start..frame_end].to_vec()).unwrap();
          let mut line = 1;

          frame_start = start;
          while frame_start > 0 {
            if let Some(b'\n') = bytes.get(frame_start - 1) {
              line += 1;
            }

            frame_start -= 1;
          }

          eprintln!("\n{} {} {}",
            colors::bold_red(">"),
            colors::bold_dim(format!("{}|", line).as_str()),
            frame);

          let prefix_width = 4 + line.to_string().len();

          eprintln!("{}{}",
            " ".repeat(prefix_width + col_index),
            colors::bold_red("^"));

          eprintln!("{}",
            colors::dim(format!("{}:{}:{}",
              &module_path,
              line,
              col_index + 1).as_str()));
        }
      }
    }
  }
}
