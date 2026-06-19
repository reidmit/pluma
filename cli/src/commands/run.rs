use crate::printing::*;
use compiler::*;

/// `pluma run <path> [args…]`. A source file is compiled to WasmGC and run on V8
/// (the deploy engine — run what you ship); a prebuilt `.wasm` runs directly.
/// Everything after the path is the program's own argv (`io.args`).
pub(crate) fn run_command(hmr: bool, entry_path: String, program_args: Vec<String>) {
	// A prebuilt WasmGC artifact (`pluma build`) runs directly under V8.
	if entry_path.ends_with(".wasm") {
		let bytes = match std::fs::read(&entry_path) {
			Ok(b) => b,
			Err(err) => {
				print_error(format!("Could not read `{}`: {}", entry_path, err));
				std::process::exit(1);
			}
		};
		// A built artifact runs from its own directory: a fullstack server reads its
		// sibling `_built/`, `public/`, and data files relative to the working dir, so
		// `pluma run out/main.wasm` has to behave as if launched from `out/`. We read the
		// bytes first (above, against the original path), then chdir into the wasm's
		// folder. The trade-off — a relative *path argument* to a CLI tool now resolves
		// against the bundle dir, not the shell's — is rare and worth the "runs from
		// anywhere" simplicity. `pluma dev` manages the working directory itself and opts
		// out via `PLUMA_RUN_NO_CHDIR`.
		if std::env::var_os("PLUMA_RUN_NO_CHDIR").is_none() {
			if let Some(parent) = std::path::Path::new(&entry_path).parent() {
				if !parent.as_os_str().is_empty() {
					let _ = std::env::set_current_dir(parent);
				}
			}
		}
		std::process::exit(host::run_streaming_v8(&bytes, &program_args));
	}

	if entry_path.ends_with(".test.pa") || entry_path.ends_with(".test") {
		print_error(format!(
			"`{}` is a test module. Use `pluma test` to run tests.",
			entry_path
		));
		std::process::exit(1);
	}

	let mut compiler = match Compiler::from_entry_path(entry_path) {
		Ok(c) => c.with_hmr(hmr),
		Err(diagnostics) => {
			print_diagnostics(diagnostics);
			std::process::exit(1);
		}
	};

	if let Err(diagnostics) = compiler.check() {
		if print_diagnostics_is_fatal(diagnostics) {
			std::process::exit(1);
		}
	}

	// Compile to a WasmGC artifact and run it under V8 — the deploy engine, the exact
	// thing `pluma build` ships ("run what you deploy"). Every builtin the language
	// exposes lowers to wasm, so a program the backend can't emit (today only the
	// web-only `std/web/dom` surface) is a hard `wasm codegen error`.
	let program = match ir::lower(&compiler) {
		Ok(p) => p,
		Err(msg) => {
			print_error(format!("ir::lower: {msg}"));
			std::process::exit(1);
		}
	};
	let bytes = match wasm::emit(&program) {
		Ok(b) => b,
		Err(diags) => {
			print_error(format!("wasm codegen error: {}", diags.0.join("; ")));
			std::process::exit(1);
		}
	};
	std::process::exit(host::run_streaming_v8(&bytes, &program_args));
}
