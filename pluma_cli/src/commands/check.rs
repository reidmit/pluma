use crate::arg_parser::ParsedArgs;
use crate::command::Command;
use crate::command_error::CommandError;
use crate::command_info::*;
use crate::errors;
use pluma_compiler::*;
use pluma_constants::*;
use std::process::exit;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct CheckCommand<'a> {
  pub entry_path: Option<String>,
  args: &'a mut ParsedArgs,
}

impl<'a> Command<'a> for CheckCommand<'a> {
  fn info() -> CommandInfo {
    CommandInfo::new("check", "Parses & type-checks a module without compiling").with_help()
  }

  fn from_inputs(args: &'a mut ParsedArgs) -> Self {
    CheckCommand {
      entry_path: args.get_positional_arg(0),
      args,
    }
  }

  fn execute(self) -> Result<(), CommandError> {
    self.args.check_valid()?;

    let compiler_options = CompilerOptions {
      entry_path: self.entry_path.unwrap_or(DEFAULT_ENTRY_FILE.to_owned()),
      mode: CompilerMode::Debug,
      output_path: None,
    };

    let mut compiler = match Compiler::from_options(compiler_options) {
      Ok(c) => c,
      Err(diagnostics) => {
        errors::print_diagnostics(None, diagnostics);
        exit(1);
      }
    };

    match compiler.check() {
      Ok(_) => {
        println!("Check succeeded without errors!");
      }

      Err(diagnostics) => {
        errors::print_diagnostics(Some(&compiler), diagnostics);
        exit(1);
      }
    }

    Ok(())
  }
}
