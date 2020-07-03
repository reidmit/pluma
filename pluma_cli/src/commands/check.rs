use crate::arg_parser::ParsedArgs;
use crate::command::Command;
use crate::command_error::CommandError;
use crate::command_info::*;
use crate::errors;
use pluma_compiler::*;
use pluma_constants::*;
use std::process::exit;

pub struct CheckCommand {}

impl Command for CheckCommand {
  fn info() -> CommandInfo {
    CommandInfo::new("check", "Parses & type-checks a module without compiling")
      .args(vec![
        Arg::new("entry", "Path to Pluma module or directory").default(DEFAULT_ENTRY_FILE)
      ])
      .flags(vec![
        Flag::with_names("parse-only", "p").description("Skip type-checking of input files")
      ])
      .with_help()
  }

  fn execute(args: &ParsedArgs) -> Result<(), CommandError> {
    let compiler_options = CompilerOptions {
      entry_path: args
        .get_positional_arg(0)
        .unwrap_or(DEFAULT_ENTRY_FILE.to_owned()),
      mode: CompilerMode::Debug,
      output_path: None,
      collect_comments: false,
    };

    let mut compiler = match Compiler::from_options(compiler_options) {
      Ok(c) => c,
      Err(diagnostics) => {
        errors::print_diagnostics(diagnostics);
        exit(1);
      }
    };

    let parse_only = args.is_flag_present("parse-only");

    let result = if parse_only {
      compiler.parse()
    } else {
      compiler.check()
    };

    match result {
      Ok(_) => {
        println!("Check succeeded without errors!");
      }

      Err(diagnostics) => {
        errors::print_diagnostics(diagnostics);
        exit(1);
      }
    }

    Ok(())
  }
}
