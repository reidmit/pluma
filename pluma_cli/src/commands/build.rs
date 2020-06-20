use crate::arg_parser::ParsedArgs;
use crate::command::*;
use crate::command_error::CommandError;
use crate::command_info::*;
use crate::errors;
use pluma_compiler::*;
use pluma_constants::*;
use std::process::exit;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct BuildCommand<'a> {
  pub entry_path: Option<String>,
  pub output_path: Option<String>,
  pub mode: Option<String>,
  args: &'a mut ParsedArgs,
}

impl<'a> Command<'a> for BuildCommand<'a> {
  fn info() -> CommandInfo {
    CommandInfo::new("build", "Compiles a module into an executable")
      .args(vec![
        Arg::new("entry", "Path to Pluma module or directory").default(DEFAULT_ENTRY_FILE)
      ])
      .flags(vec![
        Flag::with_names("out", "o")
          .description("Output executable path")
          .value_name("output"),
        Flag::with_names("mode", "m")
          .description("Optimization mode")
          .value_name("path")
          .possible_values(vec!["release", "debug"])
          .default("debug"),
      ])
      .with_help()
  }

  fn from_inputs(args: &'a mut ParsedArgs) -> Self {
    BuildCommand {
      entry_path: args.get_positional_arg(0),
      output_path: args
        .get_flag_value("out")
        .or_else(|| args.get_flag_value("o")),
      mode: args
        .get_flag_value("mode")
        .or_else(|| args.get_flag_value("m")),
      args,
    }
  }

  fn execute(self) -> Result<(), CommandError> {
    self.args.check_valid()?;

    let compiler_options = CompilerOptions {
      entry_path: self.entry_path.unwrap_or(DEFAULT_ENTRY_FILE.to_owned()),
      mode: match self.mode {
        Some(val) if val == "release" => CompilerMode::Release,
        _ => CompilerMode::Debug,
      },
      output_path: self.output_path,
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
