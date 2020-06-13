use clap::{App, AppSettings, Arg};
use pluma_compiler::compiler::Compiler;
use pluma_compiler::compiler_options::{CompilerMode, CompilerOptions};
use pluma_compiler::{BINARY_NAME, VERSION};
use std::fmt;
use std::process::exit;

mod colors;
mod diagnostics;
mod templates;

fn main() {
  let help_template = &templates::main_help_template()[..];
  let cmd_help_template = &templates::command_help_template()[..];
  let cmd_help_template_no_options = &templates::command_help_template_no_options()[..];

  let mut app = App::new(BINARY_NAME)
    .version(VERSION)
    .help_template(help_template)
    .about("Compiler & tools for the Pluma language")
    .setting(AppSettings::SubcommandRequiredElseHelp)
    .setting(AppSettings::DisableVersion)
    .setting(AppSettings::VersionlessSubcommands)
    .setting(AppSettings::AllowExternalSubcommands)
    .subcommand(
      App::new("version")
        .help_template(cmd_help_template_no_options)
        .about("Prints version and exits"),
    )
    .subcommand(
      App::new("run")
        .help_template(cmd_help_template)
        .about("Compiles & runs a module")
        .arg(
          Arg::with_name("entry")
            .about("Path to entry module or directory")
            .required(true),
        )
        .arg(
          Arg::with_name("mode")
            .about("Compiler optimization mode")
            .takes_value(true)
            .short('m')
            .long("mode")
            .default_value("debug")
            .value_name("MODE")
            .possible_values(&["debug", "release"]),
        ),
    )
    .subcommand(
      App::new("build")
        .help_template(cmd_help_template)
        .about("Compiles a module into an executable")
        .arg(
          Arg::with_name("entry")
            .about("Path to entry module or directory")
            .required(true),
        )
        .arg(
          Arg::with_name("mode")
            .about("Compiler optimization mode")
            .takes_value(true)
            .short('m')
            .long("mode")
            .default_value("debug")
            .value_name("MODE")
            .possible_values(&["debug", "release"]),
        )
        .arg(
          Arg::with_name("out")
            .about("Executable output file")
            .takes_value(true)
            .required(true)
            .short('o')
            .long("out")
            .value_name("PATH"),
        ),
    );

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
        output_path: None,
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
      println!("{}", VERSION);
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
