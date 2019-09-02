// TODO remove these, these are just for testing
#![allow(dead_code)]
#![allow(unused_imports)]

use compiler::compiler::Compiler;
use compiler::config::CompilerConfig;
use std::env;
use std::process::exit;

mod constants;
mod colors;

fn print_usage() {
  print!(
    "{bold_name} - version {version}

Compiler and tools for the Pluma language

{usage_header}
  $ {name} <command> [...options]

{commands_header}
  run, r       Build and run a given module
  help, h      Print this help text
  version, v   Print version

For help with an individual command, run:
  $ {name} <command> -h
",
    bold_name = colors::bold(constants::LANG_NAME),
    name = constants::LANG_NAME,
    version = constants::VERSION,
    usage_header = colors::bold("Usage:"),
    commands_header = colors::bold("Commands:"),
  );
}

fn print_unknown_command(command: &str) {
  print_error(format!(
    "Unknown command: {command_name}

For a full list of available commands, try:
  $ {name} help",
    command_name = command,
    name = constants::LANG_NAME,
  ));
}

fn print_error(msg: String) {
  eprintln!("{} {}", colors::bold_red("Error:"), msg);
}

fn main() {
  let config = CompilerConfig::new(Some("test".to_owned())).unwrap();
  Compiler::new(config).run().unwrap();
}

fn main2() {
  if env::args().len() > 1 {
    let command = env::args().nth(1).unwrap_or_default();

    match command.as_str() {
      "run" | "r" => {
        let given_entry_path = env::args().nth(2);
        let config = CompilerConfig::new(given_entry_path);

        match config {
          Ok(valid_config) => match Compiler::new(valid_config).run() {
            Ok(_) => {
              println!("Compilation succeeded!");
              exit(0);
            }

            Err(e) => {
              print_error(e);
              exit(1);
            }
          },

          Err(e) => {
            print_error(e);
            exit(1);
          }
        }
      }

      "help" | "h" => {
        print_usage();
        exit(0);
      }

      "version" | "v" => {
        print!("v{}", constants::VERSION);
        exit(0);
      }

      _ => {
        print_unknown_command(&command);
        exit(1);
      }
    }
  }

  print_usage();
  exit(0);
}
