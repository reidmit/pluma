use crate::arg_parser::ParsedArgs;
use crate::colors;
use crate::diagnostics;
use pluma_compiler::compiler::Compiler;
use pluma_compiler::compiler_options::*;
use pluma_compiler::BINARY_NAME;
use std::process::exit;

#[derive(Debug)]
pub struct Options {
  pub entry_path: Option<String>,
  pub mode: Option<String>,
}

pub fn extract_options(args: ParsedArgs) -> Options {
  Options {
    entry_path: args.get_positional_arg(0),
    mode: args
      .get_flag_value("mode")
      .or_else(|| args.get_flag_value("m")),
  }
}

pub fn description() -> String {
  format!("{}", "Compiles & runs a Pluma module directly.")
}

pub fn print_help() {
  println!(
    "{description}

{usage_header}
    {cmd_prefix} {binary_name} run <path> [options...]

{arguments_header}
    <path>    Path to Pluma module or directory

{options_header}
    -m, --mode    Optimization mode ('release' or 'debug', default: 'debug')
    -h, --help    Print this help text",
    description = description(),
    usage_header = colors::bold("Usage:"),
    binary_name = BINARY_NAME,
    arguments_header = colors::bold("Arguments:"),
    options_header = colors::bold("Options:"),
    cmd_prefix = colors::dim("$"),
  )
}

pub fn execute(opts: Options) {
  let options = CompilerOptions {
    entry_path: opts.entry_path.unwrap_or("main.pa".to_owned()),
    mode: match opts.mode {
      Some(val) if val == "release" => CompilerMode::Release,
      _ => CompilerMode::Debug,
    },
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
