use crate::arg_parser::ParsedArgs;
use crate::colors;
use crate::command::*;
use crate::errors;
use pluma_compiler::*;
use pluma_constants::*;
use std::process::exit;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct BuildCommand {
  pub entry_path: Option<String>,
  pub output_path: Option<String>,
  pub mode: Option<String>,
}

impl Command for BuildCommand {
  fn help_text() -> String {
    format!(
      "{binary_name} build

Compiles a module into an executable

{usage_header}
  {cmd_prefix} {binary_name} build <path> [options...]

{arguments_header}
  <path>    Path to Pluma module or directory

{options_header}
  -o, --out     Output executable path
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
    BuildCommand {
      entry_path: args.get_positional_arg(0),
      output_path: args
        .get_flag_value("out")
        .or_else(|| args.get_flag_value("o")),
      mode: args
        .get_flag_value("mode")
        .or_else(|| args.get_flag_value("m")),
    }
  }

  fn execute(self) {
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
  }
}
