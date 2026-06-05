use compiler::*;

use crate::browser_bundle;
use crate::printing::*;

/// `pluma build [--web] <file> [-o out]` — compile a module to a deploy artifact.
/// By default this lowers the shared IR for a machine/OS host through the WasmGC
/// backend and writes `<out>.wasm`, run with `pluma run <out>.wasm`. `--web` instead
/// lowers for the web/DOM sandbox and writes a browser bundle (see notes/DEPLOY.md).
pub(crate) fn build_command(
	web: bool,
	out_base: Option<String>,
	server_url: Option<String>,
	target: Option<String>,
	entry_path: String,
) {
	if target.is_some() {
		print_error(
			"`--target` was removed. Use `--web` for a browser build; omit it for a server/CLI build.",
		);
		std::process::exit(1);
	}

	// Where the generated RPC client stubs point. Default to the server's default
	// bind; `--server-url` overrides (use "" for same-origin behind a proxy).
	let server_url = server_url.unwrap_or_else(|| "http://localhost:8080".to_string());

	// FULLSTACK: a directory with both `server.pa` and `client.pa` builds two
	// artifacts from one source — `server.wasm` + a browser client bundle —
	// regardless of `--web` (each half has its own).
	if Compiler::is_fullstack_dir(&entry_path) {
		build_fullstack(entry_path, out_base, server_url);
		return;
	}

	let target = if web { Target::Web } else { Target::Sys };

	let mut compiler = match Compiler::from_entry_path(entry_path.clone()) {
		Ok(c) => c.with_target(Some(target)).with_rpc_base_url(server_url),
		Err(diagnostics) => {
			print_diagnostics(diagnostics);
			std::process::exit(1);
		}
	};
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

	let bytes = match wasm::emit_with_options(
		&program,
		wasm::EmitOptions {
			browser: target == Target::Web,
			..Default::default()
		},
	) {
		Ok(b) => b,
		Err(diags) => {
			print_error(format!("wasm codegen error: {}", diags.0.join("; ")));
			std::process::exit(1);
		}
	};
	match target {
		Target::Sys => {
			let wasm_path = format!("{base}.wasm");
			if let Err(e) = std::fs::write(&wasm_path, &bytes) {
				print_error(format!("writing {wasm_path}: {e}"));
				std::process::exit(1);
			}
			println!("wrote {wasm_path} (run with `pluma run {wasm_path}`)");
		}
		// The web bundle: the wasm artifact plus the JS loader + HTML shell that run
		// it against the real DOM. Written into a `<base>/` directory; serve it over HTTP
		// (WasmGC needs a real origin, not file://) and open `index.html`.
		Target::Web => {
			let dir = std::path::PathBuf::from(&base);
			if let Err(e) = browser_bundle::write_bundle(&dir, &bytes) {
				print_error(format!("writing web bundle to {}: {e}", dir.display()));
				std::process::exit(1);
			}
			println!(
				"wrote {0}/app.wasm, {0}/loader.js, {0}/index.html\n\
				 serve with `python3 -m http.server --directory {0}` and open the printed URL",
				dir.display()
			);
		}
	}
}

// FULLSTACK build: from one analyzed program emit two artifacts — a `server.wasm`
// (the `std.sys.http` server mounting the generated `dispatch`) and a browser client
// bundle (the generated stubs riding the host `fetch`). One `check()`, one schema
// fingerprint stamped into both, per-artifact target gating, and the emitter's
// reachability prune carves each side out of the shared IR (the server-only `remote
// def` bodies are never reached from the client's `main`, and vice versa).
fn build_fullstack(entry_path: String, out_base: Option<String>, server_url: String) {
	let mut compiler = match Compiler::from_fullstack_dir(entry_path.clone()) {
		Ok(c) => c.with_rpc_base_url(server_url),
		Err(diagnostics) => {
			print_diagnostics(diagnostics);
			std::process::exit(1);
		}
	};
	if let Err(diagnostics) = compiler.check() {
		print_diagnostics(diagnostics);
		std::process::exit(1);
	}
	// Per-artifact gating: server reachability as `sys`, client reachability as `web`.
	if let Err(diagnostics) = compiler.gate_fullstack() {
		print_diagnostics(diagnostics);
		std::process::exit(1);
	}

	let server_module = compiler.entry_modules[0].clone();
	let client_module = compiler.entry_modules[1].clone();

	let emit_one = |entry: &str, browser: bool| -> Vec<u8> {
		let program = match ir::lower_entry(&compiler, entry) {
			Ok(p) => p,
			Err(msg) => {
				print_error(format!("ir::lower: {msg}"));
				std::process::exit(1);
			}
		};
		match wasm::emit_with_options(
			&program,
			wasm::EmitOptions {
				browser,
				..Default::default()
			},
		) {
			Ok(b) => b,
			Err(diags) => {
				print_error(format!("wasm codegen error: {}", diags.0.join("; ")));
				std::process::exit(1);
			}
		}
	};

	let server_bytes = emit_one(&server_module, false);
	let client_bytes = emit_one(&client_module, true);

	// Artifacts land in an output directory (default `out/`): the server wasm
	// alongside the browser bundle (app.wasm + loader.js + index.html).
	let dir = std::path::PathBuf::from(out_base.unwrap_or_else(|| "out".to_string()));
	if let Err(e) = browser_bundle::write_bundle(&dir, &client_bytes) {
		print_error(format!("writing web bundle to {}: {e}", dir.display()));
		std::process::exit(1);
	}
	let server_path = dir.join("server.wasm");
	if let Err(e) = std::fs::write(&server_path, &server_bytes) {
		print_error(format!("writing {}: {e}", server_path.display()));
		std::process::exit(1);
	}
	println!(
		"wrote {0}/server.wasm + {0}/app.wasm, {0}/loader.js, {0}/index.html\n\
		 run the server with `pluma run {0}/server.wasm`; serve the client with\n\
		 `python3 -m http.server --directory {0}` and open the printed URL",
		dir.display()
	);
}
