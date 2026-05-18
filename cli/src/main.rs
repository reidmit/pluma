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

				vm::stdlib::register_compiler(&mut compiler);

				if let Err(diagnostics) = compiler.check() {
					print_diagnostics(diagnostics);
					std::process::exit(1);
				}

				let program = match codegen::compile(&compiler) {
					Ok(p) => p,
					Err(msg) => {
						print_error(format!("codegen error: {}", msg));
						std::process::exit(1);
					}
				};
				let mut vm_instance = vm::VM::new(program);
				if let Err(err) = vm_instance.run() {
					print_error(format!("Runtime error: {}", err.message));
					std::process::exit(1);
				}
			}

			"build" => {
				todo!()
			}

			#[cfg(debug_assertions)]
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
							println!("{:?}", token);
						}
					}

					Err(diagnostics) => {
						print_diagnostics(diagnostics);
						std::process::exit(1);
					}
				}
			}

			#[cfg(debug_assertions)]
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

				vm::stdlib::register_compiler(&mut compiler);

				match compiler.check() {
					Ok(module) => {
						println!("{:#?}", module);
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
// (deliberately) only derives in debug builds. The commands themselves are
// excluded from release builds — both as match arms above and in the help
// text below.

#[cfg(debug_assertions)]
fn print_help() {
	eprintln!(
		"{} v{}

Compiler & toolchain for the {} programming language

COMMANDS:
  run <path>       execute a module directly
  build <path>     compile a module into an executable
  tokenize <path>  dump the token stream for a module
  analyze <path>   parse, type-check & dump info about a module
  version          print compiler version info
  help             print this help text
",
		BINARY_NAME, VERSION, LANGUAGE_NAME
	)
}

#[cfg(not(debug_assertions))]
fn print_help() {
	eprintln!(
		"{} v{}

Compiler & toolchain for the {} programming language

COMMANDS:
  run <path>       execute a module directly
  build <path>     compile a module into an executable
  version          print compiler version info
  help             print this help text
",
		BINARY_NAME, VERSION, LANGUAGE_NAME
	)
}
