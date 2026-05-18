mod colors;
mod printing;

use compiler::*;
use printing::*;

fn main() {
	match std::env::args().nth(1) {
		Some(arg) => match &arg[..] {
			"run" => {
				// Parse: `run [--mode=interp|vm] <path>`
				let mut rest_args = std::env::args().skip(2);
				let mut mode = "vm";
				let mut entry_path: Option<String> = None;
				for arg in rest_args.by_ref() {
					if let Some(m) = arg.strip_prefix("--mode=") {
						mode = match m {
							"interp" | "vm" => Box::leak(m.to_string().into_boxed_str()),
							_ => {
								print_error(format!("Unknown --mode: `{}`. Expected interp or vm.", m));
								std::process::exit(1);
							}
						};
					} else if entry_path.is_none() {
						entry_path = Some(arg);
					}
				}
				let entry_path = match entry_path {
					Some(p) => p,
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

				// Both backends share the same stdlib type registration.
				vm::stdlib::register_compiler(&mut compiler);

				if let Err(diagnostics) = compiler.check() {
					print_diagnostics(diagnostics);
					std::process::exit(1);
				}

				if mode == "interp" {
					// Keep the tree-walking interpreter available as a
					// reference implementation. Note: its stdlib registration
					// is separate from the VM's.
					interpreter::stdlib::register_compiler(&mut compiler);
					let mut interp = interpreter::Interpreter::new(&compiler);
					interpreter::stdlib::register_runtime(&mut interp);
					if let Err(err) = interp.run() {
						print_error(format!("Runtime error: {}", err.message));
						std::process::exit(1);
					}
				} else {
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

				interpreter::stdlib::register_compiler(&mut compiler);

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
