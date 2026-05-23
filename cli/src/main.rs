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

			"format" => {
				let rest: Vec<String> = std::env::args().skip(2).collect();
				format_command(rest);
			}

			"test" => {
				let args: Vec<String> = std::env::args().skip(2).collect();
				test_command(args);
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

				if let Err(diagnostics) = compiler.check() {
					print_diagnostics(diagnostics);
					std::process::exit(1);
				}
				let entry_name = compiler.entry_modules.first().cloned().unwrap_or_default();
				let module = compiler.modules.get(&entry_name).unwrap();
				println!("{:#?}", module);
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

fn test_command(args: Vec<String>) {
	let cwd = match std::env::current_dir() {
		Ok(p) => p,
		Err(err) => {
			print_error(format!("Could not determine current directory: {}", err));
			std::process::exit(1);
		}
	};

	// Arg parsing: at most one positional (the starting directory) plus
	// any number of `-f <name>` filters. No positional means start at cwd.
	let mut positional: Option<String> = None;
	let mut filters: Vec<String> = Vec::new();
	let mut iter = args.into_iter();
	while let Some(a) = iter.next() {
		match a.as_str() {
			"-f" => match iter.next() {
				Some(v) => filters.push(v),
				None => {
					print_error("`-f` requires a filter argument");
					std::process::exit(1);
				}
			},
			s if s.starts_with('-') => {
				print_error(format!("unknown flag `{}`", s));
				std::process::exit(1);
			}
			_ => {
				if positional.is_some() {
					print_error("`pluma test` takes at most one directory argument");
					std::process::exit(1);
				}
				positional = Some(a);
			}
		}
	}

	let start_dir: std::path::PathBuf = match positional {
		Some(arg) => {
			let p = std::path::Path::new(&arg);
			if !p.is_dir() {
				print_error(format!("`{}` is not a directory", arg));
				std::process::exit(1);
			}
			match p.canonicalize() {
				Ok(d) => d,
				Err(err) => {
					print_error(format!("Could not resolve `{}`: {}", arg, err));
					std::process::exit(1);
				}
			}
		}
		None => cwd,
	};

	// `pluma test` requires a package root — the marker tells the runner
	// which subtree counts as "the project" and gives every `*.test.pa`
	// module a stable name to resolve `use` paths against. Without one,
	// any non-trivial test layout silently mis-resolves siblings.
	let root_dir = match compiler::find_project_root(&start_dir) {
		Some(p) => p,
		None => {
			print_error(
				"no package root found (no pluma.pa in any parent directory). \
				Create a `pluma.pa` at your project root.",
			);
			std::process::exit(1);
		}
	};

	// Module names below are paths relative to the package root, with `/`
	// flipped to `.` and the `.pa` extension stripped — so
	// `<root>/foo/bar.test.pa` becomes `foo.bar.test`.
	let mut test_modules = discover_test_modules(&root_dir);
	test_modules.sort();

	if !filters.is_empty() {
		test_modules.retain(|name| filters.iter().any(|f| name.contains(f)));
	}

	if test_modules.is_empty() {
		if filters.is_empty() {
			eprintln!(
				"no test files found (looked for *.test.pa under {})",
				root_dir.display()
			);
		} else {
			eprintln!("no test files match {:?}", filters);
		}
		return;
	}

	let count = test_modules.len();
	let module_word = if count == 1 { "module" } else { "modules" };
	if filters.is_empty() {
		println!(
			"running {} test {} in {}",
			count,
			module_word,
			root_dir.display()
		);
	} else {
		let quoted: Vec<String> = filters.iter().map(|f| format!("'{}'", f)).collect();
		let joined = match quoted.len() {
			1 => quoted[0].clone(),
			_ => {
				let (last, rest) = quoted.split_last().unwrap();
				format!("{} or {}", rest.join(", "), last)
			}
		};
		println!(
			"running {} test {} matching {} in {}",
			count,
			module_word,
			joined,
			root_dir.display()
		);
	}
	println!();

	let mut compiler = Compiler::for_root_dir(root_dir.clone());
	// Add the project marker as an entry so the analyzer type-checks
	// `def package` against `core.package.info` (catches mistakes in the
	// config even when no test code references it).
	compiler.add_entry_module(compiler::PROJECT_MARKER_MODULE.to_string());
	for name in &test_modules {
		compiler.add_entry_module(name.clone());
	}

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

	if program.tests.is_empty() {
		eprintln!("no tests found");
		return;
	}

	// Codegen iterates a HashMap, so tests come out in non-deterministic
	// module order. Sort by module name (stably) so the user sees the same
	// groupings across runs; within-module test order is source order from
	// codegen's AST walk and is preserved by the stable sort.
	let mut tests: Vec<(String, String, u32)> = program.tests.clone();
	tests.sort_by(|a, b| a.0.cmp(&b.0));
	let mut vm_instance = vm::VM::new(program);

	let total = tests.len();
	let mut passed = 0usize;
	let mut failed = 0usize;

	let mut current_module: Option<String> = None;
	for (module_name, test_name, global_idx) in tests {
		if current_module.as_deref() != Some(module_name.as_str()) {
			if current_module.is_some() {
				println!();
			}
			// Strip the redundant `.test` suffix every test module name
			// carries (e.g. `util.list-helpers.test` → `util.list-helpers`).
			let display = module_name.strip_suffix(".test").unwrap_or(&module_name);
			println!("{}", colors::bold(display));
			current_module = Some(module_name);
		}
		match vm_instance.call_test(global_idx) {
			Ok(_) => {
				println!("  {} {}", colors::bold_green("✓"), test_name);
				passed += 1;
			}
			Err(err) => {
				println!("  {} {}", colors::bold_red("✗"), test_name);
				if let (Some(module), Some(range)) = (&err.module, err.range) {
					let path = compiler::to_module_path(&root_dir, module);
					let display_path = path.strip_prefix(&root_dir).unwrap_or(&path);
					// Convert 0-indexed Range points to 1-indexed line/col so
					// the output is editor-friendly (most editors and the
					// "Cmd+click in terminal" convention expect 1-indexed).
					println!(
						"      {}:{}:{}",
						display_path.display(),
						range.start.line + 1,
						range.start.col + 1,
					);
				}
				println!("      {}", err.message);
				failed += 1;
			}
		}
	}

	println!();
	let summary = format!("{} of {} passed", passed, total);
	if failed == 0 {
		println!("{}", colors::bold_green(&summary));
	} else {
		println!("{}", colors::bold_red(&summary));
		std::process::exit(1);
	}
}

// Recursively find every `*.test.pa` file under `root` and return its module
// name (path relative to `root`, with `/` → `.` and `.pa` stripped). Hidden
// directories (anything starting with `.`) are skipped — `.git`, `.cargo`,
// etc. shouldn't be scanned.
fn discover_test_modules(root: &std::path::Path) -> Vec<String> {
	fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
		let entries = match std::fs::read_dir(dir) {
			Ok(e) => e,
			Err(_) => return,
		};
		for entry in entries.flatten() {
			let path = entry.path();
			let name = match path.file_name().and_then(|n| n.to_str()) {
				Some(n) => n,
				None => continue,
			};
			if name.starts_with('.') {
				continue;
			}
			let file_type = match entry.file_type() {
				Ok(t) => t,
				Err(_) => continue,
			};
			if file_type.is_dir() {
				walk(&path, root, out);
			} else if file_type.is_file() && name.ends_with(".test.pa") {
				if let Ok(rel) = path.strip_prefix(root) {
					let rel_str = rel.to_string_lossy();
					let stem = rel_str.strip_suffix(".pa").unwrap_or(&rel_str);
					let module_name = stem.replace(std::path::MAIN_SEPARATOR, ".");
					out.push(module_name);
				}
			}
		}
	}

	let mut out = Vec::new();
	walk(root, root, &mut out);
	out
}

fn run(entry_path: String) {
	if entry_path.ends_with(".test.pa") || entry_path.ends_with(".test") {
		print_error(format!(
			"`{}` is a test module. Use `pluma test` to run tests.",
			entry_path
		));
		std::process::exit(1);
	}

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
  format <path>... canonicalize formatting; pass `-` for stdin, `--check` to dry-run
  test [dir] [-f name]...
                   discover and run tests from `*.test.pa` files under the
                   nearest `pluma.pa`. Pass a directory to start the walk-up
                   from somewhere other than cwd. `-f name` (repeatable)
                   filters to modules whose name contains `name`.
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
  format <path>... canonicalize formatting; pass `-` for stdin, `--check` to dry-run
  test [dir] [-f name]...
                   discover and run tests from `*.test.pa` files under the
                   nearest `pluma.pa`. Pass a directory to start the walk-up
                   from somewhere other than cwd. `-f name` (repeatable)
                   filters to modules whose name contains `name`.
  version          print compiler version info
  help             print this help text
",
		BINARY_NAME, VERSION, LANGUAGE_NAME
	)
}
