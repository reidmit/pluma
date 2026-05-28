mod colors;
mod printing;

use compiler::*;
use printing::*;

fn main() {
	match std::env::args().nth(1) {
		Some(arg) => match &arg[..] {
			"run" => {
				// Pull an optional leading `--backend ir|ast` (or `--backend=ir`)
				// off the front; everything after the script path is the program's
				// own argv, surfaced through `io.args`.
				let raw: Vec<String> = std::env::args().skip(2).collect();
				let (backend, rest) = match take_backend_flag(raw) {
					Ok(pair) => pair,
					Err(msg) => {
						print_error(msg);
						std::process::exit(1);
					}
				};
				let mut rest = rest.into_iter();
				let entry_path = match rest.next() {
					Some(path) => path,
					None => {
						print_error("No module path given. Expected another argument.");
						std::process::exit(1);
					}
				};
				let program_args: Vec<String> = rest.collect();
				run(entry_path, program_args, backend);
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
			// works as shorthand for `cli run foo.pa`. Here the path is
			// argv[1], so the program's own args start at argv[2]. The backend
			// here comes only from `PLUMA_BACKEND` (no flag-stripping, so a
			// script's own `--backend` arg is left untouched).
			_ => {
				let program_args: Vec<String> = std::env::args().skip(2).collect();
				run(arg, program_args, backend_from_env());
			}
		},

		None => {
			print_help();
		}
	}
}

/// Which codegen backend `run`/`test` uses. `Ir` (the default since the
/// cutover) goes through `ir::lower` + `compile_from_ir`; `Ast` is the original
/// fused AST->bytecode walk, kept reachable via `--backend ast` /
/// `PLUMA_BACKEND=ast` as a fallback and as the differential harness's oracle.
#[derive(Clone, Copy, PartialEq)]
enum Backend {
	Ast,
	Ir,
}

/// Read the default backend from `PLUMA_BACKEND` (`ir`/`ast`), defaulting to
/// `Ir`. Only an explicit `ast` opts back into the old walk; anything else
/// (unset or unrecognized) uses `Ir`.
fn backend_from_env() -> Backend {
	match std::env::var("PLUMA_BACKEND").ok().as_deref() {
		Some("ast") => Backend::Ast,
		_ => Backend::Ir,
	}
}

/// Strip an optional leading `--backend ir|ast` / `--backend=ir` from `args`,
/// returning the chosen backend (falling back to `PLUMA_BACKEND`) and the
/// remaining args. The flag must come before the script path.
fn take_backend_flag(args: Vec<String>) -> Result<(Backend, Vec<String>), String> {
	let mut iter = args.into_iter();
	let Some(first) = iter.next() else {
		return Ok((backend_from_env(), Vec::new()));
	};
	let parse = |v: &str| match v {
		"ir" => Ok(Backend::Ir),
		"ast" => Ok(Backend::Ast),
		other => Err(format!(
			"Unknown backend `{other}` (expected `ir` or `ast`)."
		)),
	};
	if let Some(val) = first.strip_prefix("--backend=") {
		let backend = parse(val)?;
		return Ok((backend, iter.collect()));
	}
	if first == "--backend" {
		let val = iter
			.next()
			.ok_or("`--backend` expects a value (`ir` or `ast`).")?;
		let backend = parse(&val)?;
		return Ok((backend, iter.collect()));
	}
	// No flag: put the first arg back.
	let mut rest = vec![first];
	rest.extend(iter);
	Ok((backend_from_env(), rest))
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

	// Same backend selection as `run` (default `Ir`, `PLUMA_BACKEND=ast` opts
	// back to the old walk); `pluma test` takes no `--backend` flag, so it reads
	// only the env var.
	let program = match compile_program(&compiler, backend_from_env()) {
		Ok(p) => p,
		Err(msg) => {
			print_error(format!("codegen error: {}", msg));
			std::process::exit(1);
		}
	};

	let t_codegen = std::time::Instant::now();

	if program.test_suites.is_empty() {
		eprintln!("no tests found (expected a `def tests :: testing.suite` in a *.test.pa file)");
		return;
	}

	// `core.testing.new` builds the registrar the runner threads into each
	// suite. It's compiled whenever a suite is (a suite's type names it).
	let new_idx = match program.test_new {
		Some(idx) => idx,
		None => {
			print_error("internal error: `core.testing.new` was not compiled");
			std::process::exit(1);
		}
	};

	// Codegen iterates a HashMap, so suites come out in non-deterministic
	// module order. Sort by module name so the output is stable across runs.
	let mut suites: Vec<(String, u32)> = program.test_suites.clone();
	suites.sort_by(|a, b| a.0.cmp(&b.0));
	let mut vm_instance = vm::VM::new(program);

	let t_vm_new = std::time::Instant::now();
	// Split VM time: registering cases (running each suite body) vs actually
	// executing the case bodies.
	let mut register_time = std::time::Duration::ZERO;
	let mut run_time = std::time::Duration::ZERO;

	let mut passed = 0usize;
	let mut failed = 0usize;
	let mut skipped = 0usize;
	let mut todo_count = 0usize;

	for (i, (module_name, suite_idx)) in suites.iter().enumerate() {
		if i > 0 {
			println!();
		}
		// Strip the redundant `.test` suffix every test module name carries
		// (e.g. `util.list-helpers.test` → `util.list-helpers`).
		let display = module_name.strip_suffix(".test").unwrap_or(module_name);
		println!("{}", colors::bold(display));

		// Build a fresh registrar, run the suite to register its cases, then
		// drain the flat list of entries.
		let t_reg = std::time::Instant::now();
		let entries = match run_suite(&mut vm_instance, new_idx, *suite_idx) {
			Ok(entries) => entries,
			Err(err) => {
				println!(
					"  {} failed to load suite: {}",
					colors::bold_red("✗"),
					err.message
				);
				failed += 1;
				continue;
			}
		};
		register_time += t_reg.elapsed();

		// Focus: if any case is focused, only focused cases run.
		let any_focused = entries
			.iter()
			.any(|e| field_variant(e, "status").as_deref() == Some("focused"));

		let mut printed_path: Vec<String> = Vec::new();
		for entry in &entries {
			let name = field_string(entry, "name").unwrap_or_default();
			let path = field_string_list(entry, "path");
			let status = field_variant(entry, "status").unwrap_or_else(|| "normal".to_string());
			print_group_headers(&mut printed_path, &path);
			let indent = "  ".repeat(path.len() + 1);

			let should_run = match status.as_str() {
				"pending" => {
					println!(
						"{}{} {} {}",
						indent,
						colors::bold_yellow("○"),
						name,
						colors::dim("(todo)")
					);
					todo_count += 1;
					false
				}
				"skipped" => {
					println!(
						"{}{} {} {}",
						indent,
						colors::dim("-"),
						name,
						colors::dim("(skipped)")
					);
					skipped += 1;
					false
				}
				"focused" => true,
				_ => {
					if any_focused {
						println!(
							"{}{} {} {}",
							indent,
							colors::dim("-"),
							name,
							colors::dim("(not focused)")
						);
						skipped += 1;
						false
					} else {
						true
					}
				}
			};

			if !should_run {
				continue;
			}

			let body = match field(entry, "body") {
				Some(b) => b,
				None => continue,
			};
			let t_case = std::time::Instant::now();
			let case_result = vm_instance.call_function(body, vec![vm::Value::Nothing]);
			run_time += t_case.elapsed();
			match case_result {
				// `ok ()` — the case passed.
				Ok(result) if variant_of(&result).as_deref() == Some("ok") => {
					println!("{}{} {}", indent, colors::bold_green("✓"), name);
					passed += 1;
				}
				// `err message` — one or more assertions failed.
				Ok(result) => {
					println!("{}{} {}", indent, colors::bold_red("✗"), name);
					let msg = variant_payload_string(&result).unwrap_or_default();
					for line in msg.lines() {
						println!("{}    {}", indent, line);
					}
					failed += 1;
				}
				// A genuine runtime error (e.g. `io.fail`, div-by-zero) — the
				// case crashed rather than producing a result.
				Err(err) => {
					println!(
						"{}{} {} {}",
						indent,
						colors::bold_red("✗"),
						name,
						colors::dim("(errored)")
					);
					if let (Some(module), Some(range)) = (&err.module, err.range) {
						let p = compiler::to_module_path(&root_dir, module);
						let display_path = p.strip_prefix(&root_dir).unwrap_or(&p);
						// 0-indexed Range → 1-indexed line/col for editor links.
						println!(
							"{}    {}:{}:{}",
							indent,
							display_path.display(),
							range.start.line + 1,
							range.start.col + 1,
						);
					}
					println!("{}    {}", indent, err.message);
					failed += 1;
				}
			}
		}
	}

	println!();
	let total = passed + failed;
	let mut summary = format!("{} of {} passed", passed, total);
	if skipped > 0 {
		summary.push_str(&format!(", {} skipped", skipped));
	}
	if todo_count > 0 {
		summary.push_str(&format!(", {} todo", todo_count));
	}
	if timing {
		let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
		let t_vm_end = std::time::Instant::now();
		eprintln!();
		eprintln!("── timing (PLUMA_TIMING) ──────────────");
		eprintln!("  discover+setup : {:>8.2} ms", ms(t_setup - t_start));
		eprintln!("  check          : {:>8.2} ms", ms(t_check - t_setup));
		eprintln!("  codegen        : {:>8.2} ms", ms(t_codegen - t_check));
		eprintln!("  vm::new        : {:>8.2} ms", ms(t_vm_new - t_codegen));
		eprintln!("  vm register    : {:>8.2} ms", ms(register_time));
		eprintln!(
			"  vm run cases   : {:>8.2} ms ({} cases)",
			ms(run_time),
			total
		);
		eprintln!("  ─────────────────────────────────────");
		eprintln!("  total in-proc  : {:>8.2} ms", ms(t_vm_end - t_start));
	}

	if failed == 0 {
		println!("{}", colors::bold_green(&summary));
	} else {
		println!("{}", colors::bold_red(&summary));
		std::process::exit(1);
	}
}

// Build a fresh registrar (via `core.testing.new`), run `suite` to register
// its cases into it, then drain and return the flat entry list. Errors here
// mean the suite's registration code itself crashed.
fn run_suite(
	vm: &mut vm::VM,
	new_idx: u32,
	suite_idx: u32,
) -> Result<Vec<vm::Value>, vm::RuntimeError> {
	let new_fn = vm.force_global(new_idx)?;
	let registrar = vm.call_function(new_fn, vec![vm::Value::Nothing])?;
	let suite_fn = vm.force_global(suite_idx)?;
	vm.call_function(suite_fn, vec![registrar.clone()])?;
	let drain = field(&registrar, "drain")
		.ok_or_else(|| vm::RuntimeError::new("registrar is missing a `drain` field"))?;
	match vm.call_function(drain, vec![vm::Value::Nothing])? {
		vm::Value::List(xs) => Ok(xs.iter().cloned().collect()),
		_ => Err(vm::RuntimeError::new("`drain` did not return a list")),
	}
}

// Print group headers for any groups newly entered going from the previously
// printed path to `path`, updating `printed` in place. Indents by depth so the
// tree nests under the module header.
fn print_group_headers(printed: &mut Vec<String>, path: &[String]) {
	let common = printed
		.iter()
		.zip(path.iter())
		.take_while(|(a, b)| a == b)
		.count();
	printed.truncate(common);
	for seg in &path[common..] {
		let indent = "  ".repeat(printed.len() + 1);
		println!("{}{}", indent, colors::bold_dim(seg));
		printed.push(seg.clone());
	}
}

// --- small readers over the runtime `Value` tree the registrar produces ---

fn field(v: &vm::Value, name: &str) -> Option<vm::Value> {
	match v {
		vm::Value::Record(m) => m.get(name).cloned(),
		_ => None,
	}
}

fn variant_of(v: &vm::Value) -> Option<String> {
	match v {
		vm::Value::Variant(d) => Some(d.variant.as_str().to_string()),
		_ => None,
	}
}

fn variant_payload_string(v: &vm::Value) -> Option<String> {
	match v {
		vm::Value::Variant(d) => d.payload.first().map(|p| format!("{}", p)),
		_ => None,
	}
}

fn field_variant(v: &vm::Value, name: &str) -> Option<String> {
	field(v, name).as_ref().and_then(variant_of)
}

fn field_string(v: &vm::Value, name: &str) -> Option<String> {
	match field(v, name) {
		Some(vm::Value::String(s)) => Some(s.as_str().to_string()),
		_ => None,
	}
}

fn field_string_list(v: &vm::Value, name: &str) -> Vec<String> {
	match field(v, name) {
		Some(vm::Value::List(xs)) => xs
			.iter()
			.filter_map(|x| match x {
				vm::Value::String(s) => Some(s.as_str().to_string()),
				_ => None,
			})
			.collect(),
		_ => Vec::new(),
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

/// Compile a checked `Compiler` to a runnable `vm::Program` via the chosen
/// backend. `Ir` (the default) lowers to the mid-level IR first; `Ast` is the
/// original fused walk, still reachable as a fallback.
fn compile_program(compiler: &Compiler, backend: Backend) -> Result<vm::Program, String> {
	match backend {
		Backend::Ir => {
			let program = ir::lower(compiler).map_err(|e| format!("ir::lower: {e}"))?;
			codegen::compile_from_ir(&program).map_err(|e| e.to_string())
		}
		Backend::Ast => codegen::compile(compiler).map_err(|e| e.to_string()),
	}
}

fn run(entry_path: String, program_args: Vec<String>, backend: Backend) {
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

	let program = match compile_program(&compiler, backend) {
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
  [run] [--backend ir|ast] <path>
                   execute a module directly (the `run` keyword is optional).
                   `--backend` selects the codegen path (default `ir`; also
                   reads `PLUMA_BACKEND`)
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
  [run] [--backend ir|ast] <path>
                   execute a module directly (the `run` keyword is optional).
                   `--backend` selects the codegen path (default `ir`; also
                   reads `PLUMA_BACKEND`)
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
