mod browser_bundle;
mod cli;
mod colors;
mod commands;
mod printing;

use clap::Parser;

use cli::{Cli, Command};

fn main() {
	match Cli::parse().command {
		Command::Version => println!("v{}", compiler::VERSION),

		Command::Run {
			hmr,
			path,
			program_args,
		} => commands::run::run_command(hmr, path, program_args),

		Command::Build {
			web,
			out,
			server_url,
			target,
			path,
		} => commands::build::build_command(web, out, server_url, target, path),

		Command::Dev {
			web,
			port,
			server_url,
			path,
		} => commands::dev::dev_command(web, port, server_url, path),

		Command::Format { check, paths } => commands::format::format_command(check, paths),

		Command::Lint { fix, paths } => commands::lint::lint_command(fix, paths),

		Command::Test { filters, dir } => commands::test::test_command(filters, dir),

		Command::LanguageServer => lsp::run(),

		#[cfg(debug_assertions)]
		Command::Analyze { path } => commands::analyze::analyze_command(path),

		#[cfg(debug_assertions)]
		Command::Tokenize { path } => commands::tokenize::tokenize_command(path),

		// `pluma <path> [args…]`: an unrecognized first token is taken as a module
		// path to run, so `pluma foo.pa` is shorthand for `pluma run foo.pa`. The
		// path is the captured token; the rest is the program's own argv.
		Command::External(args) => {
			let mut args = args.into_iter();
			let path = args.next().expect("external subcommand always has a token");
			commands::run::run_command(false, path, args.collect());
		}
	}
}
