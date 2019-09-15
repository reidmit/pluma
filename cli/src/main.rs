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

fn print_usage() {
  print!(
    "{bold_name} - version {version}

Compiler and tools for the Pluma language

{usage_header}
  $ {cli_name} <command> [...options]

{commands_header}
  run       Build and run a given module
  help      Print this usage information
  version   Print version

For help with an individual command, try:
  $ {cli_name} <command> -h
",
    bold_name = colors::bold("pluma"),
    cli_name = "pluma",
    version = VERSION,
    usage_header = colors::bold("Usage:"),
    commands_header = colors::bold("Commands:"),
  );
}

fn main() {
  match options::parse_options() {
    Ok(Command::Help) => {
      print_usage();
      exit(0);
    },

    Ok(Command::Version) => {
      println!("v{}", VERSION);
      exit(0);
    },

    Ok(Command::Run { root_dir, entry_path }) => {
      let mut compiler = Compiler::new(CompilerConfig {
        root_dir,
        entry_path
      });

      match compiler.run() {
        Ok(_) => {
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

  for (module_name, module_errors) in summary.module_errors {
    for ModuleCompilationErrorDetail { module_path, location, message } in module_errors {
      eprintln!("{} in {}:\n",
        colors::bold_red("Error"),
        colors::bold(module_name.as_str()),
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
