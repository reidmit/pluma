use crate::arg_parser::ParsedArgs;

pub trait Command {
  fn help_text() -> String;

  fn from_inputs(args: ParsedArgs) -> Self;

  fn execute(self);

  fn print_help() {
    println!("{}", Self::help_text());
  }
}
