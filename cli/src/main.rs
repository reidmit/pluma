use pluma_compiler::compiler::{Compiler, CompilerConfig};
use pluma_compiler::error_formatter::ErrorFormatter;
use pluma_compiler::errors::PackageCompilationErrorSummary;
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
          print_error_summary(error_formatter.get_error_summary());
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

fn print_error_summary(summary: PackageCompilationErrorSummary) {
  if !summary.package_errors.is_empty() {
    for package_error in summary.package_errors {
      print_error(package_error);
    }

    return
  }

  for (module_path, module_errors) in summary.module_errors {
    let message = format!("in {}: {:#?}", module_path, module_errors);

    print_error(message);
  }
}
