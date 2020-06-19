use crate::arg_parser::ParsedArgs;
use crate::colors;
use crate::command::Command;
use crate::errors;
use pluma_compiler::*;
use pluma_constants::*;
use std::process::exit;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct RunCommand {
  pub entry_path: Option<String>,
  pub mode: Option<String>,
}

impl Command for RunCommand {
  fn help_text() -> String {
    format!(
      "{binary_name} run

Compiles & runs a Pluma module directly

{usage_header}
  {cmd_prefix} {binary_name} run <path> [options...]

{arguments_header}
  <path>    Path to Pluma module or directory

{options_header}
  -m, --mode    Optimization mode ('release' or 'debug', default: 'debug')
  -h, --help    Print this help text",
      usage_header = colors::bold("Usage:"),
      binary_name = BINARY_NAME,
      arguments_header = colors::bold("Arguments:"),
      options_header = colors::bold("Options:"),
      cmd_prefix = colors::dim("$"),
    )
  }

  fn from_inputs(args: ParsedArgs) -> Self {
    RunCommand {
      entry_path: args.get_positional_arg(0),
      mode: args
        .get_flag_value("mode")
        .or_else(|| args.get_flag_value("m")),
    }
  }

  fn execute(self) {
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
