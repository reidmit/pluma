// TODO remove these, these are just for testing
#![allow(dead_code)]
#![allow(unused_imports)]

use pluma_compiler::compiler::Compiler;
use pluma_compiler::config::CompilerConfig;
use std::env;
use std::process::exit;

mod constants;
mod colors;

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
    version = constants::VERSION,
    usage_header = colors::bold("Usage:"),
    commands_header = colors::bold("Commands:"),
  );
}

fn print_unknown_command(command: &str) {
  print_error(format!(
    "Unknown command: {command_name}

For a full list of available commands, try:
  $ {cli_name} help",
    command_name = command,
    cli_name = "pluma",
  ));
}

fn print_error(msg: String) {
  eprintln!("{} {}", colors::bold_red("Error:"), msg);
}

fn main() {
  if env::args().len() > 1 {
    let command = env::args().nth(1).unwrap_or_default();

    match command.as_str() {
      "run" => {
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

      "help" => {
        print_usage();
        exit(0);
      }

      "version" => {
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
