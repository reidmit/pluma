use crate::arg_parser::ParsedArgs;
use crate::command::Command;
use crate::command_error::CommandError;
use crate::command_info::*;
use crate::errors;
use pluma_compiler::*;
use pluma_constants::*;
use pluma_doc::*;
use std::process::exit;

pub struct DocCommand {}

impl Command for DocCommand {
  fn info() -> CommandInfo {
    CommandInfo::new("doc", "Generates documentation pages")
      .args(vec![
        Arg::new("entry", "Path to Pluma module or directory").default(DEFAULT_ENTRY_FILE)
      ])
      .flags(vec![
        Flag::with_names("serve", "s").description("Start a live-reloading local server"),
        Flag::with_names("port", "p")
          .description("Port to use for local server")
          .default("4700"),
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
      collect_comments: true,
    };

    let mut compiler = match Compiler::from_options(compiler_options) {
      Ok(c) => c,
      Err(diagnostics) => {
        errors::print_diagnostics(diagnostics);
        exit(1);
      }
    };

    match compiler.check() {
      Ok(_) => {}
      Err(diagnostics) => {
        errors::print_diagnostics(diagnostics);
        exit(1);
      }
    }

    let mut doc_builder = DocBuilder::new(compiler);

    doc_builder.build();

    Ok(())
  }
}
