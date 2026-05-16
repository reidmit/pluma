mod colors;
mod printing;

use compiler::*;
use printing::*;

fn main() {
	match std::env::args().nth(1) {
		Some(arg) => match &arg[..] {
			"run" => {
				let entry_path = match std::env::args().nth(2) {
					Some(path) => path,
					None => {
						print_error("No module path given. Expected another argument.");
						std::process::exit(1);
					}
				};

				let mut compiler = match Compiler::from_entry_path(entry_path) {
					Ok(c) => c,
					Err(diagnostics) => {
						print_diagnostics(diagnostics);
						std::process::exit(1);
					}
				};

				if let Err(diagnostics) = compiler.check() {
					print_diagnostics(diagnostics);
					std::process::exit(1);
				}

				let interp = interpreter::Interpreter::new(&compiler);
				if let Err(err) = interp.run() {
					print_error(format!("Runtime error: {}", err.message));
					std::process::exit(1);
				}
			}

			"build" => {
				todo!()
			}

			"tokenize" => {
				let entry_path = match std::env::args().nth(2) {
					Some(path) => path,
					None => {
						print_error("No module path given. Expected another argument.");
						std::process::exit(1);
					}
				};

				let mut compiler = match Compiler::from_entry_path(entry_path) {
					Ok(c) => c,
					Err(diagnostics) => {
						print_diagnostics(diagnostics);
						std::process::exit(1);
					}
				};

				match compiler.tokenize() {
					Ok(tokens) => {
						for token in tokens {
							print_token(&token);
						}
					}

					Err(diagnostics) => {
						print_diagnostics(diagnostics);
						std::process::exit(1);
					}
				}
			}

			"analyze" => {
				let entry_path = match std::env::args().nth(2) {
					Some(path) => path,
					None => {
						print_error("No module path given. Expected another argument.");
						std::process::exit(1);
					}
				};

				let mut compiler = match Compiler::from_entry_path(entry_path) {
					Ok(c) => c,
					Err(diagnostics) => {
						print_diagnostics(diagnostics);
						std::process::exit(1);
					}
				};

				match compiler.check() {
					Ok(module) => {
						print_module(module);
					}

					Err(diagnostics) => {
						print_diagnostics(diagnostics);
						std::process::exit(1);
					}
				}
			}

			"help" => {
				print_help();
			}

			"version" => {
				println!("v{}", VERSION)
			}

			other => {
				print_error(format!("Unrecognized command: `{}`\n", other));
				print_help();
				std::process::exit(1);
			}
		},

		None => {
			print_help();
		}
	}
}

// `tokenize` and `analyze` dump Debug-formatted output, which the codebase
// (deliberately) only derives in debug builds. Gate the dumpers accordingly;
// in release we surface a clear error rather than printing nothing.

#[cfg(debug_assertions)]
fn print_token(token: &Token) {
	println!("{:?}", token);
}

#[cfg(not(debug_assertions))]
fn print_token(_: &Token) {
	print_error("`tokenize` requires a debug build (Debug impls are gated on debug_assertions).");
	std::process::exit(1);
}

#[cfg(debug_assertions)]
fn print_module(module: &Module) {
	println!("{:#?}", module);
}

#[cfg(not(debug_assertions))]
fn print_module(_: &Module) {
	print_error("`analyze` requires a debug build (Debug impls are gated on debug_assertions).");
	std::process::exit(1);
}

fn print_help() {
	eprintln!(
		"{} v{}

Compiler & toolchain for the {} programming language

COMMANDS:
  run <path>       execute a module directly
  build <path>     compile a module into an executable
  analyze <path>   parse, type-check & dump info about a module
  version          print compiler version info
  help             print this help text
",
		BINARY_NAME, VERSION, LANGUAGE_NAME
	)
}
