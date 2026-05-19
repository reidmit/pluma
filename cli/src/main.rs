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
				run(entry_path);
			}

			"build" => {
				todo!()
			}

			"format" => {
				let rest: Vec<String> = std::env::args().skip(2).collect();
				format_command(rest);
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

			// Anything else is treated as a path to run, so `cli foo.pa`
			// works as shorthand for `cli run foo.pa`.
			_ => {
				run(arg);
			}
		},

		None => {
			print_help();
		}
	}
}

fn run(entry_path: String) {
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

fn format_command(args: Vec<String>) {
	let mut check = false;
	let mut paths: Vec<String> = Vec::new();
	for a in args {
		match a.as_str() {
			"--check" => check = true,
			_ => paths.push(a),
		}
	}

	if paths.is_empty() {
		print_error("No path given. Expected a file path or `-` for stdin.");
		std::process::exit(1);
	}

	let mut any_changed = false;

	for path in &paths {
		if path == "-" {
			let mut input = Vec::new();
			if let Err(err) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut input) {
				print_error(format!("Failed to read stdin: {}", err));
				std::process::exit(1);
			}
			match formatter::format_source(&input) {
				Ok(out) => {
					if check {
						if out.as_bytes() != input.as_slice() {
							any_changed = true;
						}
					} else {
						print!("{}", out);
					}
				}
				Err(diagnostics) => {
					print_diagnostics(diagnostics);
					std::process::exit(1);
				}
			}
			continue;
		}

		let bytes = match std::fs::read(path) {
			Ok(b) => b,
			Err(err) => {
				print_error(format!("Could not read `{}`: {}", path, err));
				std::process::exit(1);
			}
		};

		match formatter::format_source(&bytes) {
			Ok(out) => {
				let changed = out.as_bytes() != bytes.as_slice();
				if check {
					if changed {
						any_changed = true;
						eprintln!("would reformat {}", path);
					}
				} else if changed {
					if let Err(err) = std::fs::write(path, out.as_bytes()) {
						print_error(format!("Could not write `{}`: {}", path, err));
						std::process::exit(1);
					}
				}
			}
			Err(_diagnostics) => {
				// Skip unparseable files rather than aborting — the user may
				// be formatting a batch that includes intentionally-broken
				// fixtures (e.g. analyzer error tests).
				eprintln!("skipping {} (parse error)", path);
			}
		}
	}

	if check && any_changed {
		std::process::exit(1);
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
  [run] <path>     execute a module directly (the `run` keyword is optional)
  build <path>     compile a module into an executable
  format <path>... canonicalize formatting; pass `-` for stdin, `--check` to dry-run
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
  [run] <path>     execute a module directly (the `run` keyword is optional)
  build <path>     compile a module into an executable
  format <path>... canonicalize formatting; pass `-` for stdin, `--check` to dry-run
  version          print compiler version info
  help             print this help text
",
		BINARY_NAME, VERSION, LANGUAGE_NAME
	)
}
