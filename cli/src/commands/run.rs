use compiler::*;

use crate::printing::*;

/// `pluma run <path> [args…]`. A source file is compiled to WasmGC and run on V8
/// (the deploy engine — run what you ship); a prebuilt `.wasm` runs directly.
/// Everything after the path is the program's own argv (`io.args`).
pub(crate) fn run_command(args: Vec<String>) {
	let mut entry_path: Option<String> = None;
	let mut program_args: Vec<String> = Vec::new();
	for a in args {
		if entry_path.is_none() && a == "--vm" {
			print_error("The `--vm` flag has been removed — `pluma run` uses V8 (the deploy engine).");
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

pub(crate) fn run(entry_path: String, program_args: Vec<String>) {
	// A prebuilt WasmGC artifact (`pluma build`) runs directly under V8.
	if entry_path.ends_with(".wasm") {
		let bytes = match std::fs::read(&entry_path) {
			Ok(b) => b,
			Err(err) => {
				print_error(format!("Could not read `{}`: {}", entry_path, err));
				std::process::exit(1);
			}
		};
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

	// Compile to a WasmGC artifact and run it under V8 — the deploy engine, the exact
	// thing `pluma build` ships ("run what you deploy"). Every builtin the language
	// exposes lowers to wasm, so a program the backend can't emit (today only the
	// web-only `std.web.dom` surface) is a hard `wasm codegen error`.
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
