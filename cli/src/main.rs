mod browser_bundle;
mod colors;
mod commands;
mod printing;

use compiler::VERSION;

fn main() {
	let argv: Vec<String> = std::env::args().collect();

	match argv.get(1).map(String::as_str) {
		Some("run") => commands::run::run_command(argv[2..].to_vec()),
		Some("build") => commands::build::build_command(argv[2..].to_vec()),
		Some("dev") => commands::dev::dev_command(argv[2..].to_vec()),
		Some("format") => commands::format::format_command(argv[2..].to_vec()),
		Some("test") => commands::test::test_command(argv[2..].to_vec()),

		#[cfg(debug_assertions)]
		Some("tokenize") => commands::tokenize::tokenize_command(argv[2..].to_vec()),
		#[cfg(debug_assertions)]
		Some("analyze") => commands::analyze::analyze_command(argv[2..].to_vec()),

		Some("help") => commands::help::print_help(),
		Some("version") => println!("v{}", VERSION),

		// Anything else is treated as a path to run, so `pluma foo.pa` works as
		// shorthand for `pluma run foo.pa` (on V8). Here the path is argv[1], so the
		// program's own args start at argv[2].
		Some(arg) => commands::run::run(arg.to_string(), argv[2..].to_vec()),

		None => commands::help::print_help(),
	}
}
