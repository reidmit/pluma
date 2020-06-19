use crate::arg_parser::ParsedArgs;
use crate::colors;
use crate::command::Command;
use crate::errors;
use pluma_compiler::*;
use pluma_constants::*;
use std::process::exit;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct CheckCommand {
  pub entry_path: Option<String>,
}

impl Command for CheckCommand {
  fn help_text() -> String {
    format!(
      "{binary_name} check

Parses & type-checks a module without compiling

{usage_header}
  {cmd_prefix} {binary_name} check <path> [options...]

{arguments_header}
  <path>    Path to Pluma module or directory

{options_header}
  -h, --help    Print this help text",
      usage_header = colors::bold("Usage:"),
      binary_name = BINARY_NAME,
      arguments_header = colors::bold("Arguments:"),
      options_header = colors::bold("Options:"),
      cmd_prefix = colors::dim("$"),
    )
  }

  fn from_inputs(args: ParsedArgs) -> Self {
    CheckCommand {
      entry_path: args.get_positional_arg(0),
    }
  }

  fn execute(self) {
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
  }
}
