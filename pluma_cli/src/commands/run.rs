use crate::arg_parser::ParsedArgs;
use crate::command::Command;
use crate::command_error::CommandError;
use crate::command_info::*;
use crate::errors;
use pluma_compiler::*;
use pluma_constants::*;
use std::process::exit;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct RunCommand<'a> {
  pub entry_path: Option<String>,
  pub mode: Option<String>,
  args: &'a mut ParsedArgs,
}

impl<'a> Command<'a> for RunCommand<'a> {
  fn info() -> CommandInfo {
    CommandInfo::new("run", "Compiles & runs a Pluma module directly").with_help()
  }

  fn from_inputs(args: &'a mut ParsedArgs) -> Self {
    RunCommand {
      entry_path: args.get_positional_arg(0),
      mode: args
        .get_flag_value("mode")
        .or_else(|| args.get_flag_value("m")),
      args,
    }
  }

  fn execute(self) -> Result<(), CommandError> {
    self.args.check_valid()?;

    let options = CompilerOptions {
      entry_path: self.entry_path.unwrap_or(DEFAULT_ENTRY_FILE.to_owned()),
      mode: match self.mode {
        Some(val) if val == "release" => CompilerMode::Release,
        _ => CompilerMode::Debug,
      },
      output_path: None,
    };

    let mut compiler = match Compiler::from_options(options) {
      Ok(c) => c,
      Err(diagnostics) => {
        errors::print_diagnostics(None, diagnostics);
        exit(1);
      }
    };

    match compiler.run() {
      Ok(exit_code) => {
        exit(exit_code);
      }

      Err(diagnostics) => {
        errors::print_diagnostics(Some(&compiler), diagnostics);
        exit(1);
      }
    }
  }
}
