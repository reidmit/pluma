use crate::arg_parser::ParsedArgs;
use crate::command::Command;
use crate::command_error::*;
use crate::command_info::*;
use crate::errors::print_diagnostics;
use glob::{glob_with, MatchOptions};
use pluma_constants::*;
use pluma_formatter::*;

pub struct FormatCommand {}

impl Command for FormatCommand {
  fn info() -> CommandInfo {
    CommandInfo::new("format", "Re-formats Pluma code")
      .args(vec![
        Arg::new("pattern", "Pattern of files to format").default("**/*.pa")
      ])
      .flags(vec![])
      .with_help()
  }

  fn execute(args: &ParsedArgs) -> Result<(), CommandError> {
    let pattern = args.get_positional_arg(0).unwrap_or("**/*.pa".to_owned());

    let glob_options = MatchOptions {
      case_sensitive: false,
      require_literal_leading_dot: false,
      require_literal_separator: false,
    };

    let mut paths = Vec::new();

    match glob_with(&pattern, glob_options) {
      Ok(entries) => {
        for entry in entries {
          if let Ok(path) = entry {
            if path.extension().unwrap_or_default() == FILE_EXTENSION {
              paths.push(path);
            }
          }
        }
      }

      Err(_) => {
        return Err(CommandError {
          command: FormatCommand::info().name.to_owned(),
          kind: CommandErrorKind::InvalidFilePatternArgument(pattern),
        })
      }
    }

    let formatter = Formatter::new(&paths);

    if let Err(diagnostics) = formatter.format() {
      print_diagnostics(diagnostics);
    }

    Ok(())
  }
}
