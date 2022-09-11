mod colors;
mod printing;

use compiler::*;
use printing::*;

fn main() {
  match std::env::args().nth(1) {
    Some(arg) => match &arg[..] {
      "run" => {
        todo!()
      }

      "build" => {
        todo!()
      }

      "analyze" => {
        let entry_path = match std::env::args().nth(2) {
          Some(path) => path,
          None => {
            print_error("No module path given. Expected another argument.");
            std::process::exit(1);
          }
        };

        let mut compiler = match Compiler::from_entry_path(entry_path) {
          Ok(c) => c,
          Err(diagnostics) => {
            print_diagnostics(diagnostics);
            std::process::exit(1);
          }
        };

        match compiler.check() {
          Ok(module) => {
            println!("{:#?}", module);
          }

          Err(diagnostics) => {
            print_diagnostics(diagnostics);
            std::process::exit(1);
          }
        }
      }

      "help" => {
        print_help();
      }

      other => {
        print_error(format!("Unrecognized command: `{}`\n", other));
        print_help();
        std::process::exit(1);
      }
    },

    None => {
      print_help();
    }
  }
}

fn print_help() {
  eprintln!(
    "{} v{}

Compiler & toolchain for the {} programming language

COMMANDS:
  run <path>     execute a module directly
  build <path>   compile a module into an executable
  analyze        parse, type-check & dump info about a module
  help           print this help text
",
    BINARY_NAME, VERSION, LANGUAGE_NAME
  )
}
