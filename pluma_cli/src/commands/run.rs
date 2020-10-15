use crate::arg_parser::ParsedArgs;
use crate::command::Command;
use crate::command_error::CommandError;
use crate::command_info::*;
use crate::errors;
use pluma_compiler::*;
use pluma_constants::*;
use std::process::exit;

pub struct RunCommand {}

impl Command for RunCommand {
  fn info() -> CommandInfo {
    CommandInfo::new("run", "Compiles & runs a Pluma module directly")
      .args(vec![
        Arg::new("entry", "Path to Pluma module or directory").default(DEFAULT_ENTRY_FILE)
      ])
      .flags(vec![Flag::with_names("mode", "m")
        .description("Optimization mode")
        .single_value()
        .value_name("path")
        .possible_values(vec!["release", "debug"])
        .default("debug")])
      .with_help()
  }

  fn execute(args: &ParsedArgs) -> Result<(), CommandError> {
    let options = CompilerOptions {
      entry_path: args
        .get_positional_arg(0)
        .unwrap_or(DEFAULT_ENTRY_FILE.to_owned()),
      mode: match args.get_flag_value("mode") {
        Some(val) if val == "release" => CompilerMode::Release,
        _ => CompilerMode::Debug,
      },
      output_path: None,
    };

    let mut compiler = match Compiler::from_options(options) {
      Ok(c) => c,
      Err(diagnostics) => {
        errors::print_diagnostics(diagnostics);
        exit(1);
      }
    };

    match compiler.run() {
      Ok(exit_code) => {
        exit(exit_code);
      }

      Err(diagnostics) => {
        errors::print_diagnostics(diagnostics);
        exit(1);
      }
    }
  }
}
