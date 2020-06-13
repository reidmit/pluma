use clap::{App, AppSettings, Arg};
use pluma_compiler::compiler::Compiler;
use pluma_compiler::{LANG_NAME, VERSION};
use std::fmt;
use std::process::exit;

mod colors;
mod diagnostics;
mod templates;

fn main() {
  let help_template = &templates::main_help_template()[..];
  let cmd_help_template = &templates::command_help_template()[..];

  let mut app = App::new(LANG_NAME)
    .version(VERSION)
    .help_template(help_template)
    .about("Compiler & tools for the Pluma language")
    .setting(AppSettings::SubcommandRequiredElseHelp)
    .setting(AppSettings::DisableVersion)
    .setting(AppSettings::VersionlessSubcommands)
    .setting(AppSettings::AllowExternalSubcommands)
    .subcommand(
      App::new("version")
        .help_template(cmd_help_template)
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
        ),
    );

  let matches = app.get_matches_mut();

  match matches.subcommand() {
    ("run", input) => {
      let entry_path = input.unwrap().value_of("entry").unwrap().to_owned();

      let mut compiler = match Compiler::from_path(entry_path) {
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

    ("build", input) => {
      let entry_path = input.unwrap().value_of("entry").unwrap().to_owned();

      let mut compiler = match Compiler::from_path(entry_path) {
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

fn print_error<T: fmt::Display>(msg: T, suggest_help: bool) {
  eprintln!("{} {}", colors::bold_red("Error:"), msg);

  if suggest_help {
    eprintln!(
      "\nFor a list of available commands and flags, try:\n    {cmd_prefix} {lang_name} help",
      cmd_prefix = colors::dim("$"),
      lang_name = LANG_NAME
    )
  }
}
