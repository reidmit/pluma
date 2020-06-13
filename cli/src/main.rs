use clap::{App, AppSettings, Arg};
use pluma_compiler::{BINARY_NAME, VERSION};

mod colors;
mod diagnostics;
mod repl;
mod runner;
mod templates;

fn main() {
  let help_template = &templates::main_help_template()[..];
  let cmd_help_template = &templates::command_help_template()[..];
  let cmd_help_template_no_options = &templates::command_help_template_no_options()[..];

  let mut app = App::new(BINARY_NAME)
    .version(VERSION)
    .help_template(help_template)
    .about("Compiler & tools for the Pluma language")
    .setting(AppSettings::SubcommandRequiredElseHelp)
    .setting(AppSettings::DisableVersion)
    .setting(AppSettings::VersionlessSubcommands)
    .setting(AppSettings::AllowExternalSubcommands)
    .subcommand(
      App::new("version")
        .help_template(cmd_help_template_no_options)
        .about("Prints version and exits"),
    )
    .subcommand(
      App::new("run")
        .help_template(cmd_help_template)
        .about("Compiles & runs a module")
        .arg(
          Arg::with_name("entry")
            .about("Path to entry module or directory")
            .required(true),
        )
        .arg(
          Arg::with_name("mode")
            .about("Compiler optimization mode")
            .takes_value(true)
            .short('m')
            .long("mode")
            .default_value("debug")
            .value_name("MODE")
            .possible_values(&["debug", "release"]),
        ),
    )
    .subcommand(
      App::new("build")
        .help_template(cmd_help_template)
        .about("Compiles a module into an executable")
        .arg(
          Arg::with_name("entry")
            .about("Path to entry module or directory")
            .required(true),
        )
        .arg(
          Arg::with_name("mode")
            .about("Compiler optimization mode")
            .takes_value(true)
            .short('m')
            .long("mode")
            .default_value("debug")
            .value_name("MODE")
            .possible_values(&["debug", "release"]),
        )
        .arg(
          Arg::with_name("out")
            .about("Executable output file")
            .takes_value(true)
            .required(true)
            .short('o')
            .long("out")
            .value_name("PATH"),
        ),
    )
    .subcommand(
      App::new("repl")
        .help_template(cmd_help_template)
        .about("Starts an interactive REPL session"),
    );

  runner::run(&mut app);
}
