use crate::arg_parser::ParsedArgs;
use crate::colors;
use crate::errors;
use pluma_compiler::compiler::Compiler;
use pluma_compiler::compiler_options::*;
use pluma_compiler::BINARY_NAME;
use std::process::exit;

pub struct Options {
  pub entry_path: Option<String>,
}

pub fn extract_options(args: ParsedArgs) -> Options {
  Options {
    entry_path: args.get_positional_arg(0),
  }
}

pub fn description() -> String {
  format!("{}", "Parses & type-checks a module without compiling")
}

pub fn print_help() {
  println!(
    "{description}

{usage_header}
    {cmd_prefix} {binary_name} check <path> [options...]

{arguments_header}
    <path>    Path to Pluma module or directory

{options_header}
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
  let compiler_options = CompilerOptions {
    entry_path: opts.entry_path.unwrap_or("main.pa".to_owned()),
    mode: CompilerMode::Debug,
    output_path: None,
    execute_after_compilation: false,
  };

  let mut compiler = match Compiler::from_options(compiler_options) {
    Ok(c) => c,
    Err(diagnostics) => {
      errors::print_diagnostics(None, diagnostics);
      exit(1);
    }
  };

  match compiler.compile() {
    Ok(_) => {
      println!("Compilation succeeded!");
    }

    Err(diagnostics) => {
      errors::print_diagnostics(Some(&compiler), diagnostics);
      exit(1);
    }
  }
}
