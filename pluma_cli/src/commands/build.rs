use crate::arg_parser::ParsedArgs;
use crate::command::*;
use crate::command_error::CommandError;
use crate::command_info::*;
use crate::errors;
use pluma_compiler::*;
use pluma_constants::*;
use std::process::exit;

pub struct BuildCommand {}

impl Command for BuildCommand {
  fn info() -> CommandInfo {
    CommandInfo::new("build", "Compiles a module into an executable")
      .args(vec![
        Arg::new("entry", "Path to Pluma module or directory").default(DEFAULT_ENTRY_FILE)
      ])
      .flags(vec![
        Flag::with_names("out", "o")
          .description("Output executable path")
          .single_value()
          .value_name("output"),
        Flag::with_names("mode", "m")
          .description("Optimization mode")
          .single_value()
          .value_name("path")
          .possible_values(vec!["release", "debug"])
          .default("debug"),
      ])
      .with_help()
  }

  fn execute(args: &ParsedArgs) -> Result<(), CommandError> {
    let compiler_options = CompilerOptions {
      entry_path: args
        .get_positional_arg(0)
        .unwrap_or(DEFAULT_ENTRY_FILE.to_owned()),

      mode: match args.get_flag_value("mode") {
        Some(val) if val == "release" => CompilerMode::Release,
        _ => CompilerMode::Debug,
      },

      output_path: args.get_flag_value("out"),
    };

    let mut compiler = match Compiler::from_options(compiler_options) {
      Ok(c) => c,
      Err(diagnostics) => {
        errors::print_diagnostics(None, diagnostics);
        exit(1);
      }
    };

    match compiler.emit() {
      Ok(_) => {
        println!("Compilation succeeded!");
      }

      Err(diagnostics) => {
        errors::print_diagnostics(Some(&compiler), diagnostics);
        exit(1);
      }
    }

    Ok(())
  }
}
