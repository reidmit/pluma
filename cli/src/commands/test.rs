use compiler::*;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::printing::*;
use crate::watch::{POLL_INTERVAL, scan};

pub(crate) fn test_command(filters: Vec<String>, watch: bool, dir: Option<String>) {
	let cwd = match std::env::current_dir() {
		Ok(p) => p,
		Err(err) => {
			print_error(format!("Could not determine current directory: {}", err));
			std::process::exit(1);
		}
	};

	// No directory given means start the walk-up from cwd.
	let start_dir: PathBuf = match dir {
		Some(arg) => {
			let p = Path::new(&arg);
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
			print_error("No package root found. Create a `pluma.pa` in your root directory.");
			std::process::exit(1);
		}
	};

	if watch {
		watch_suite(&filters, &root_dir);
	} else {
		std::process::exit(run_suite(&filters, &root_dir));
	}
}

/// Re-run the suite on every source change, never returning. The initial run
/// happens immediately; thereafter a cheap mtime fingerprint is polled and a
/// change triggers a fresh run. Compile and test failures print and keep the
/// loop alive — the point of watch mode is to fix-and-rerun without restarting.
fn watch_suite(filters: &[String], root_dir: &Path) -> ! {
	let clear = std::io::stdout().is_terminal();

	loop {
		if clear {
			// Clear the screen and scrollback so each run reads as the whole
			// picture, not a scroll of stale output.
			print!("\x1b[2J\x1b[3J\x1b[H");
		}
		run_suite(filters, root_dir);
		println!();
		println!("watching for changes — press ctrl-c to exit");

		// Baseline taken after the run, so anything the suite itself touched on
		// disk doesn't read as a change and retrigger immediately.
		let baseline = scan(root_dir);
		while scan(root_dir) == baseline {
			std::thread::sleep(POLL_INTERVAL);
		}
	}
}

/// Discover, compile, and run the suite once, returning the exit code the
/// process should carry (0 = all passed). Diagnostics and errors are printed
/// here rather than aborting, so a caller in watch mode can run again.
fn run_suite(filters: &[String], root_dir: &Path) -> i32 {
	// PLUMA_TIMING=1 prints a per-phase wall-clock breakdown to stderr.
	let timing = std::env::var("PLUMA_TIMING").is_ok();
	let t_start = std::time::Instant::now();
	let root_dir = root_dir.to_path_buf();

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
		return 0;
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
	// `def package` against `std/package.info` (catches mistakes in the
	// config even when no test code references it).
	compiler.add_entry_module(compiler::PROJECT_MARKER_MODULE.to_string());
	for name in &test_modules {
		compiler.add_entry_module(name.clone());
	}

	let t_setup = std::time::Instant::now();

	if let Err(diagnostics) = compiler.check() {
		if print_diagnostics_is_fatal(diagnostics) {
			return 1;
		}
	}

	let t_check = std::time::Instant::now();

	// Synthesize a test entry over the discovered suites and emit a WasmGC module,
	// then run it under V8 (the deploy engine — `pluma test` exercises the exact
	// artifact you ship). The runner is itself Pluma: `std/test.run-all` flattens
	// each suite, runs the cases, prints the tree, and returns ok / err.
	let use_color = std::io::stdout().is_terminal();
	let program = match ir::lower_tests(&compiler, use_color) {
		Ok(p) => p,
		Err(msg) => {
			print_error(format!("ir::lower: {msg}"));
			return 1;
		}
	};

	if program.test_suites.is_empty() {
		eprintln!("no tests found (expected a `def tests :: test.suite` in a *.test.pa file)");
		return 0;
	}

	let bytes = match wasm::emit(&program) {
		Ok(b) => b,
		Err(diags) => {
			print_error(format!("wasm codegen error: {}", diags.0.join("; ")));
			return 1;
		}
	};

	let t_codegen = std::time::Instant::now();

	// Run each test module in its own fresh isolate, in parallel over the
	// once-compiled module. The exit code reflects pass/fail.
	let code = host::run_test_v8(&bytes, program.test_suites.len(), use_color);

	// Wall-clock for the whole command (discover + compile + run), printed under
	// the Pluma-rendered summary line so every `pluma test` ends with how long it
	// took. `PLUMA_TIMING` breaks this down per phase; this is the at-a-glance number.
	let style = crate::colors::Style::detect();
	println!(
		"{}",
		style.dim(&format!(
			"finished in {:.2}s",
			t_start.elapsed().as_secs_f64()
		))
	);

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

	code
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
					let module_name = stem.replace(std::path::MAIN_SEPARATOR, "/");
					out.push(module_name);
				}
			}
		}
	}

	let mut out = Vec::new();
	walk(root, root, &mut out);
	out
}
