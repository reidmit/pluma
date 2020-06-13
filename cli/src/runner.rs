use crate::colors;
use crate::diagnostics;
use crate::repl;
use clap::App;
use pluma_compiler::compiler::Compiler;
use pluma_compiler::compiler_options::{CompilerMode, CompilerOptions};
use pluma_compiler::{BINARY_NAME, VERSION};
use std::fmt;
use std::process::exit;

pub fn run(app: &mut App) {
  let matches = app.get_matches_mut();

  match matches.subcommand() {
    ("run", input) => {
      let options = CompilerOptions {
        entry_path: input.unwrap().value_of("entry").unwrap().to_owned(),
        mode: input.unwrap().value_of("mode").map(str_to_mode).unwrap(),
        output_path: None,
        execute_after_compilation: true,
      };

      let mut compiler = match Compiler::from_options(options) {
        Ok(c) => c,
        Err(diagnostics) => {
          diagnostics::print(None, diagnostics);
          exit(1);
        }
      };

      match compiler.compile() {
        Ok(None) => {
          println!("Compilation succeeded!");
        }

        Ok(Some(exit_code)) => {
          exit(exit_code);
        }

        Err(diagnostics) => {
          diagnostics::print(Some(&compiler), diagnostics);
          exit(1);
        }
      }
    }

    ("build", input) => {
      let options = CompilerOptions {
        entry_path: input.unwrap().value_of("entry").unwrap().to_owned(),
        mode: input.unwrap().value_of("mode").map(str_to_mode).unwrap(),
        output_path: input.unwrap().value_of("out").map(|s| s.to_owned()),
        execute_after_compilation: false,
      };

      let mut compiler = match Compiler::from_options(options) {
        Ok(c) => c,
        Err(diagnostics) => {
          diagnostics::print(None, diagnostics);
          exit(1);
        }
      };

      match compiler.compile() {
        Ok(_) => {
          println!("Compilation succeeded!");
        }

        Err(diagnostics) => {
          diagnostics::print(Some(&compiler), diagnostics);
          exit(1);
        }
      }
    }

    ("version", _) => {
      println!("pluma version {}", VERSION);
    }

    ("repl", _) => {
      repl::run();
    }

    (unknown_command, _) => {
      print_error(
        format!("Command '{}' is not recognized.", unknown_command),
        true,
      );

      exit(1);
    }
  }
}

fn str_to_mode(str_val: &str) -> CompilerMode {
  if str_val == "release" {
    CompilerMode::Release
  } else {
    CompilerMode::Debug
  }
}

fn print_error<T: fmt::Display>(msg: T, suggest_help: bool) {
  eprintln!("{} {}", colors::bold_red("Error:"), msg);

  if suggest_help {
    eprintln!(
      "\nFor a list of available commands and flags, try:\n    {cmd_prefix} {lang_name} help",
      cmd_prefix = colors::dim("$"),
      lang_name = BINARY_NAME
    )
  }
}
