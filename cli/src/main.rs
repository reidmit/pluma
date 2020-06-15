mod arg_parser;
mod colors;
mod commands;
mod diagnostics;

fn main() {
  let args = std::env::args().skip(1).collect();
  let parsed_args = arg_parser::parse_args(args);

  match &parsed_args.subcommand()[..] {
    "build" => {
      if parsed_args.is_help_requested() {
        commands::build::print_help();
      } else {
        let opts = commands::build::extract_options(parsed_args);
        commands::build::execute(opts);
      }
    }

    "check" => {
      if parsed_args.is_help_requested() {
        commands::check::print_help();
      } else {
        let opts = commands::check::extract_options(parsed_args);
        commands::check::execute(opts);
      }
    }

    "run" => {
      if parsed_args.is_help_requested() {
        commands::run::print_help();
      } else {
        let opts = commands::run::extract_options(parsed_args);
        commands::run::execute(opts);
      }
    }

    "repl" => {
      if parsed_args.is_help_requested() {
        commands::repl::print_help();
      } else {
        commands::repl::execute();
      }
    }

    "version" => {
      if parsed_args.is_help_requested() {
        commands::version::print_help();
      } else {
        commands::version::execute();
      }
    }

    "help" => {
      if parsed_args.is_help_requested() {
        commands::help::print_help();
      } else {
        match parsed_args.get_positional_arg(0) {
          Some(val) => match &val[..] {
            "build" => commands::build::print_help(),
            "run" => commands::run::print_help(),
            "help" => commands::help::print_help(),
            "repl" => commands::repl::print_help(),
            "version" => commands::version::print_help(),
            _ => commands::help::execute(),
          },

          _ => commands::help::execute(),
        }
      }
    }

    _ => commands::help::execute(),
  }
}
