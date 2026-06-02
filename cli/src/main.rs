mod colors;
mod printing;
mod repl;

use compiler::*;
use printing::*;

fn main() {
	match std::env::args().nth(1) {
		Some(arg) => match &arg[..] {
			"run" => {
				// `pluma run <path> [args…]`. A source file is compiled to WasmGC and run
				// on V8 (the deploy engine — run what you ship); a prebuilt `.wasm` runs
				// directly. Everything after the path is the program's own argv (`io.args`).
				let mut entry_path: Option<String> = None;
				let mut program_args: Vec<String> = Vec::new();
				for a in std::env::args().skip(2) {
					if entry_path.is_none() && a == "--vm" {
						print_error(
							"The `--vm` flag has been removed — `pluma run` uses V8 (the deploy engine).",
						);
						std::process::exit(1);
					} else if entry_path.is_none() {
						entry_path = Some(a);
					} else {
						program_args.push(a);
					}
				}
				let entry_path = match entry_path {
					Some(path) => path,
					None => {
						print_error("No module path given. Expected another argument.");
						std::process::exit(1);
					}
				};
				run(entry_path, program_args);
			}

			"repl" => {
				let rest: Vec<String> = std::env::args().skip(2).collect();
				repl::repl_command(rest);
			}

			"build" => {
				let rest: Vec<String> = std::env::args().skip(2).collect();
				build_command(rest);
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
			// works as shorthand for `cli run foo.pa` (on V8). Here the path is argv[1],
			// so the program's own args start at argv[2].
			_ => {
				let program_args: Vec<String> = std::env::args().skip(2).collect();
				run(arg, program_args);
			}
		},

		None => {
			print_help();
		}
	}
}

fn test_command(args: Vec<String>) {
	// PLUMA_TIMING=1 prints a per-phase wall-clock breakdown to stderr.
	let timing = std::env::var("PLUMA_TIMING").is_ok();
	let t_start = std::time::Instant::now();
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

	let t_setup = std::time::Instant::now();

	if let Err(diagnostics) = compiler.check() {
		print_diagnostics(diagnostics);
		std::process::exit(1);
	}

	let t_check = std::time::Instant::now();

	// Synthesize a test entry over the discovered suites and emit a WasmGC module,
	// then run it under V8 (the deploy engine — `pluma test` exercises the exact
	// artifact you ship). The runner is itself Pluma: `core.test.run-all` flattens
	// each suite, runs the cases, prints the tree, and returns ok / err.
	let use_color = std::io::IsTerminal::is_terminal(&std::io::stdout());
	let program = match ir::lower_tests(&compiler, use_color) {
		Ok(p) => p,
		Err(msg) => {
			print_error(format!("ir::lower: {msg}"));
			std::process::exit(1);
		}
	};

	if program.test_suites.is_empty() {
		eprintln!("no tests found (expected a `def tests :: test.suite` in a *.test.pa file)");
		return;
	}

	let bytes = match wasm::emit(&program) {
		Ok(b) => b,
		Err(diags) => {
			print_error(format!("wasm codegen error: {}", diags.0.join("; ")));
			std::process::exit(1);
		}
	};

	let t_codegen = std::time::Instant::now();

	// The runner streams the report to stdout; the exit code reflects pass/fail.
	let code = host::run_test_v8(&bytes);

	if timing {
		let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
		let t_end = std::time::Instant::now();
		eprintln!();
		eprintln!("── timing (PLUMA_TIMING) ──────────────");
		eprintln!("  discover+setup : {:>8.2} ms", ms(t_setup - t_start));
		eprintln!("  check          : {:>8.2} ms", ms(t_check - t_setup));
		eprintln!("  lower+emit     : {:>8.2} ms", ms(t_codegen - t_check));
		eprintln!("  run (v8)       : {:>8.2} ms", ms(t_end - t_codegen));
		eprintln!("  ─────────────────────────────────────");
		eprintln!("  total          : {:>8.2} ms", ms(t_end - t_start));
	}

	std::process::exit(code);
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

/// Compile a checked `Compiler` to a runnable `vm::Program`: lower to the
/// mid-level IR, then emit bytecode from it.
fn compile_program(compiler: &Compiler) -> Result<vm::Program, String> {
	let mut program = ir::lower(compiler).map_err(|e| format!("ir::lower: {e}"))?;
	ir::optimize(&mut program);
	codegen::compile_from_ir(&program).map_err(|e| e.to_string())
}

/// `pluma build [--target server|browser] <file> [-o out]` — compile a module to
/// a deploy artifact. `--target server` (the default) lowers the shared IR under
/// `Platform::Server` through the WasmGC backend and writes `<out>.wasm`, run with
/// `pluma run <out>.wasm`. `--target browser` is reserved for the WasmGC frontend
/// (see notes/DEPLOY.md) and is not yet available.
fn build_command(args: Vec<String>) {
	let mut entry_path: Option<String> = None;
	let mut out_base: Option<String> = None;
	let mut target = String::from("server");
	let mut iter = args.into_iter();
	while let Some(a) = iter.next() {
		match a.as_str() {
			"--target" => {
				if let Some(t) = iter.next() {
					target = t;
				}
			}
			"-o" | "--out" => out_base = iter.next(),
			_ => entry_path = Some(a),
		}
	}
	let entry_path = match entry_path {
		Some(p) => p,
		None => {
			print_error("No module path given. Expected another argument.");
			std::process::exit(1);
		}
	};

	let platform = match target.as_str() {
		"server" => Platform::Server,
		"browser" => {
			print_error(
				"The browser target is not yet available — the WasmGC frontend is pending (see notes/DEPLOY.md). Use `--target server` to emit a WasmGC artifact.",
			);
			std::process::exit(1);
		}
		other => {
			print_error(format!(
				"Unknown --target `{other}`. Expected `server` or `browser`."
			));
			std::process::exit(1);
		}
	};

	let mut compiler = match Compiler::from_entry_path(entry_path.clone()) {
		Ok(c) => c.with_platform(platform),
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

	let program = match ir::lower(&compiler) {
		Ok(p) => p,
		Err(msg) => {
			print_error(format!("ir::lower: {msg}"));
			std::process::exit(1);
		}
	};

	// Default the output base name to the entry file's stem.
	let base = out_base.unwrap_or_else(|| {
		std::path::Path::new(&entry_path)
			.file_stem()
			.and_then(|s| s.to_str())
			.unwrap_or("out")
			.to_string()
	});

	match platform {
		Platform::Server => {
			let bytes = match wasm::emit(&program) {
				Ok(b) => b,
				Err(diags) => {
					print_error(format!("wasm codegen error: {}", diags.0.join("; ")));
					std::process::exit(1);
				}
			};
			let wasm_path = format!("{base}.wasm");
			if let Err(e) = std::fs::write(&wasm_path, &bytes) {
				print_error(format!("writing {wasm_path}: {e}"));
				std::process::exit(1);
			}
			println!("wrote {wasm_path} (run with `pluma run {wasm_path}`)");
		}
		// `--target browser` exits earlier in this fn (WasmGC frontend pending);
		// only `Platform::Server` reaches the emit.
		_ => unreachable!("non-server build target should have exited before emit"),
	}
}

fn run(entry_path: String, program_args: Vec<String>) {
	// A prebuilt WasmGC artifact (`pluma build`) runs directly under V8.
	if entry_path.ends_with(".wasm") {
		let bytes = match std::fs::read(&entry_path) {
			Ok(b) => b,
			Err(err) => {
				print_error(format!("Could not read `{}`: {}", entry_path, err));
				std::process::exit(1);
			}
		};
		std::process::exit(host::run_streaming_v8(&bytes));
	}

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

	// Compile to a WasmGC artifact and run it under V8 — the same thing `pluma build`
	// ships. A few builtins aren't yet wasm host imports (notably `io.args`); for a
	// program the wasm backend can't emit, we fall back to the bytecode VM so it still
	// runs. This is an internal capability bridge, not a user-selectable engine.
	if let Ok(bytes) = emit_wasm(&compiler) {
		std::process::exit(host::run_streaming_v8(&bytes));
	}

	let program = match compile_program(&compiler) {
		Ok(p) => p,
		Err(msg) => {
			print_error(format!("codegen error: {}", msg));
			std::process::exit(1);
		}
	};
	let mut vm_instance = vm::VM::new(program).with_args(program_args);
	if let Err(err) = vm_instance.run() {
		if err.is_user_abort {
			// A deliberate bail (`io.fail` / `expect`): the message is the
			// program's own, so print it bare — no `Runtime error:` prefix,
			// which we reserve for genuine VM faults.
			eprintln!("{}", err.message);
		} else {
			print_error(format!("Runtime error: {}", err.message));
		}
		std::process::exit(1);
	}
}

/// Lower a checked `Compiler` to the raw mid-level IR and emit a WasmGC artifact
/// (the deploy backend runs its own pipeline off the raw IR — no `ir::optimize`,
/// matching `pluma build`). `Err` ⇒ the wasm backend can't yet handle this program.
fn emit_wasm(compiler: &Compiler) -> Result<Vec<u8>, ()> {
	let program = ir::lower(compiler).map_err(|_| ())?;
	wasm::emit(&program).map_err(|_| ())
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
  repl [--dump]    start an interactive REPL session; with `--dump` (or when
                   stdin is piped) read submissions from stdin instead
  build <path> [--target server] [-o out]
                   compile a module to a WasmGC deploy artifact (.wasm); run it
                   with `pluma run <out>.wasm`
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
  repl [--dump]    start an interactive REPL session; with `--dump` (or when
                   stdin is piped) read submissions from stdin instead
  build <path> [--target server] [-o out]
                   compile a module to a WasmGC deploy artifact (.wasm); run it
                   with `pluma run <out>.wasm`
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
