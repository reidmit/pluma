use compiler::*;

use crate::browser_bundle;
use crate::printing::*;

/// `pluma build <dir> [-o out]` — compile a project directory into `out/`. The mode is
/// read from the directory's entry files: a `main.pa` builds a CLI/server (`out/main.wasm`),
/// a `client.pa` builds a static browser site, and both together build a fullstack app
/// (server + the client bundle it serves).
pub(crate) fn build_command(
	web: bool,
	out_base: Option<String>,
	server_url: Option<String>,
	optimize: Option<String>,
	target: Option<String>,
	entry_path: String,
) {
	if target.is_some() {
		print_error(
			"`--target` was removed. The build mode comes from the directory's entry files — \
			 a `main.pa`, a `client.pa`, or both.",
		);
		std::process::exit(1);
	}
	if web {
		print_error(
			"`--web` was removed. A directory with a `client.pa` (and no `main.pa`) builds a \
			 static site; that's the browser build now.",
		);
		std::process::exit(1);
	}

	let start = std::time::Instant::now();

	// Post-optimize the emitted wasm with Binaryen's wasm-opt. On by default at
	// `-O3` (the pass is a code-size win and never slower at runtime); `-O <level>`
	// overrides, and `-O 0` opts out.
	let opt_level = match optimize.as_deref() {
		None => Some(wasm::OptLevel::O3),
		Some("0") => None,
		Some(s) => match wasm::OptLevel::parse(s) {
			Some(l) => Some(l),
			None => {
				print_error(format!(
					"unknown -O level {s:?}; use 2/3/4 for speed, s/z for size, or 0 to skip"
				));
				std::process::exit(1);
			}
		},
	};

	// Where the generated RPC client stubs point. Default to the server's default
	// bind; `--server-url` overrides (use "" for same-origin behind a proxy).
	let server_url = server_url.unwrap_or_else(|| "http://localhost:8080".to_string());

	// The build mode is read from the directory's entry files — there is no mode flag.
	// `main.pa` → a CLI or standalone server (one wasm); `client.pa` → a static browser
	// site; both → a fullstack app (server + client).
	let dir = std::path::Path::new(&entry_path);
	if !dir.is_dir() {
		print_error(format!(
			"`pluma build` takes a project directory, not a file — one with a `main.pa` \
			 (CLI/server), a `client.pa` (static site), or both (fullstack). Got `{entry_path}`."
		));
		std::process::exit(1);
	}
	let has_main = dir.join("main.pa").is_file();
	let has_client = dir.join("client.pa").is_file();
	match (has_main, has_client) {
		(true, true) => build_fullstack(entry_path, out_base, server_url, opt_level, start),
		(true, false) => build_sys(entry_path, out_base, server_url, opt_level, start),
		(false, true) => build_static(entry_path, out_base, server_url, opt_level, start),
		(false, false) => {
			print_error(format!(
				"`{entry_path}` has no `main.pa` or `client.pa` — `pluma build` needs at \
				 least one (both, for a fullstack app)."
			));
			std::process::exit(1);
		}
	}
}

/// Lower the checked program to WasmGC and run the optional wasm-opt pass; `browser`
/// selects the web/DOM emit profile. Exits the process on a lowering or codegen error.
fn lower_and_emit(
	compiler: &Compiler,
	browser: bool,
	opt_level: Option<wasm::OptLevel>,
) -> Vec<u8> {
	let program = match ir::lower(compiler) {
		Ok(p) => p,
		Err(msg) => {
			print_error(format!("ir::lower: {msg}"));
			std::process::exit(1);
		}
	};
	let bytes = match wasm::emit_with_options(
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
	};
	run_wasm_opt(bytes, opt_level)
}

/// CLI / standalone-server build: a directory with a `main.pa` compiles to a single
/// `out/main.wasm`, run with `pluma run out/main.wasm`.
fn build_sys(
	entry_path: String,
	out_base: Option<String>,
	server_url: String,
	opt_level: Option<wasm::OptLevel>,
	start: std::time::Instant,
) {
	let mut compiler = match Compiler::from_entry_path(entry_path) {
		Ok(c) => c
			.with_target(Some(Target::Sys))
			.with_rpc_base_url(server_url),
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
	let bytes = lower_and_emit(&compiler, false, opt_level);

	let out_dir = std::path::PathBuf::from(out_base.unwrap_or_else(|| "out".to_string()));
	let wasm_path = out_dir.join("main.wasm");
	if let Err(e) = std::fs::create_dir_all(&out_dir) {
		print_error(format!("creating {}: {e}", out_dir.display()));
		std::process::exit(1);
	}
	if let Err(e) = std::fs::write(&wasm_path, &bytes) {
		print_error(format!("writing {}: {e}", wasm_path.display()));
		std::process::exit(1);
	}
	print_build_summary(
		&format!("app → {}/", out_dir.display()),
		&[Artifact::file(&wasm_path, None)],
		&[(format!("pluma run {}", wasm_path.display()), "run it")],
		start.elapsed(),
	);
}

/// Static-site build: a directory with a `client.pa` (and no `main.pa`) compiles a
/// browser bundle into `out/` — `index.html` + `loader.js` + `app.wasm`. Serve it from
/// any static host over HTTP (WasmGC needs a real origin, not `file://`). A static site
/// has no backend, so a `remote def` in it is rejected — nothing would answer the call.
fn build_static(
	entry_path: String,
	out_base: Option<String>,
	server_url: String,
	opt_level: Option<wasm::OptLevel>,
	start: std::time::Instant,
) {
	let mut compiler = match Compiler::from_entry_path(format!("{entry_path}/client")) {
		Ok(c) => c
			.with_target(Some(Target::Web))
			.with_rpc_base_url(server_url),
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
	// No server means no RPC: a `remote def` here has no endpoint to reach. Reject it
	// rather than emit a client that fetches a route nothing serves. Adding a `main.pa`
	// makes it a fullstack app, where the server answers these.
	if !compiler.rpc_endpoints.is_empty() {
		print_error(
			"a static site has no backend, so `remote def` isn't allowed in a \
			 `client.pa`-only build. Add a `main.pa` (making it a fullstack app) to serve \
			 these calls.",
		);
		std::process::exit(1);
	}
	let bytes = lower_and_emit(&compiler, true, opt_level);

	let out_dir = std::path::PathBuf::from(out_base.unwrap_or_else(|| "out".to_string()));
	if let Err(e) = browser_bundle::write_bundle(&out_dir, &bytes) {
		print_error(format!("writing web bundle to {}: {e}", out_dir.display()));
		std::process::exit(1);
	}
	print_build_summary(
		&format!("static site → {}/", out_dir.display()),
		&[
			Artifact::file(&out_dir.join("index.html"), None),
			Artifact::file(&out_dir.join("app.wasm"), None),
			Artifact::file(&out_dir.join("loader.js"), None),
		],
		&[(
			format!("python3 -m http.server --directory {}", out_dir.display()),
			"serve it, then open the printed URL",
		)],
		start.elapsed(),
	);
}

/// Apply the optional `wasm-opt` pass. A `None` level is a no-op; a wasm-opt failure
/// is non-fatal — we warn and keep the unoptimized bytes so the build still produces a
/// runnable artifact.
fn run_wasm_opt(bytes: Vec<u8>, level: Option<wasm::OptLevel>) -> Vec<u8> {
	let Some(level) = level else {
		return bytes;
	};
	match wasm::optimize(&bytes, level) {
		Ok(opt) => opt,
		Err(e) => {
			print_error(format!("wasm-opt failed ({e}); using unoptimized module"));
			bytes
		}
	}
}

/// One line in the build summary: a written file (or asset directory), its on-disk
/// size (or a count like "2 assets"), an optional dim note (e.g. "extracted from SSR"),
/// and how many files it actually represents (1 for a file, N for an asset directory).
struct Artifact {
	path: String,
	size: String,
	note: Option<String>,
	files: usize,
}

impl Artifact {
	/// A file artifact, sizing it from disk (a missing/unreadable file reads as 0 B —
	/// the summary is cosmetic, never worth failing the build over).
	fn file(path: &std::path::Path, note: Option<String>) -> Self {
		Artifact {
			path: path.display().to_string(),
			size: human_size(std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)),
			note,
			files: 1,
		}
	}
}

/// Render a byte count as a short human-readable size (`480 B`, `5.9 KB`, `1.2 MB`).
fn human_size(bytes: u64) -> String {
	const KB: f64 = 1024.0;
	let n = bytes as f64;
	if n < KB {
		format!("{bytes} B")
	} else if n < KB * KB {
		format!("{:.1} KB", n / KB)
	} else {
		format!("{:.1} MB", n / (KB * KB))
	}
}

/// Print the build summary: a header naming what was built, a column-aligned list of
/// the written artifacts, a `next:` block of the commands to run them, and a footer
/// with the total file count and elapsed time. Replaces the older interleaved prose so
/// everything the build produced lands in one ordered block.
fn print_build_summary(
	title: &str,
	artifacts: &[Artifact],
	next: &[(String, &str)],
	elapsed: std::time::Duration,
) {
	let s = crate::colors::Style::detect();

	// Align the size column to the widest path and the note column to the widest size.
	let path_w = artifacts.iter().map(|a| a.path.len()).max().unwrap_or(0);
	let size_w = artifacts.iter().map(|a| a.size.len()).max().unwrap_or(0);

	let mut o = format!("\n  {}\n\n", s.bold(title));
	for a in artifacts {
		o += &format!(
			"    {:<path_w$}   {:>size_w$}",
			s.cyan(&a.path),
			a.size,
			// Pad against the raw (uncolored) widths; the ANSI codes add invisible bytes.
			path_w = path_w + s.cyan(&a.path).len() - a.path.len(),
			size_w = size_w,
		);
		if let Some(note) = &a.note {
			o += &format!("   {}", s.dim(note));
		}
		o.push('\n');
	}

	if !next.is_empty() {
		o += &format!("\n  {}\n", s.dim("next:"));
		let cmd_w = next.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
		for (cmd, desc) in next {
			o += &format!(
				"    {:<cmd_w$}   {}\n",
				s.bold(cmd),
				s.dim(desc),
				cmd_w = cmd_w + s.bold(cmd).len() - cmd.len(),
			);
		}
	}

	let n: usize = artifacts.iter().map(|a| a.files).sum();
	o += &format!(
		"\n  {}\n",
		s.dim(&format!(
			"built {n} file{} in {}",
			if n == 1 { "" } else { "s" },
			human_duration(elapsed),
		))
	);
	print!("{o}");
}

/// Render a build duration as `230ms` under a second, or `1.3s` at or above it.
fn human_duration(d: std::time::Duration) -> String {
	let ms = d.as_millis();
	if ms < 1000 {
		format!("{ms}ms")
	} else {
		format!("{:.1}s", d.as_secs_f64())
	}
}

// FULLSTACK build: from one analyzed program emit two artifacts — a `main.wasm`
// (the `std/sys/http` server mounting the generated `dispatch`) and a browser client
// bundle (the generated stubs riding the host `fetch`). One `check()`, one schema
// fingerprint stamped into both, per-artifact target gating, and the emitter's
// reachability prune carves each side out of the shared IR (the server-only `remote
// def` bodies are never reached from the client's `main`, and vice versa).
fn build_fullstack(
	entry_path: String,
	out_base: Option<String>,
	server_url: String,
	opt_level: Option<wasm::OptLevel>,
	start: std::time::Instant,
) {
	let mut compiler = match Compiler::from_fullstack_dir(entry_path.clone()) {
		Ok(c) => c.with_rpc_base_url(server_url),
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

	let server_bytes = run_wasm_opt(emit_one(&server_module, false), opt_level);
	let client_bytes = run_wasm_opt(emit_one(&client_module, true), opt_level);

	// Artifacts land in an output directory (default `out/`): the server wasm and the
	// client hydration bundle it serves. The server *is* the app — it SSRs each page,
	// so there's no standalone HTML/JS shell, only the `_built/` bundle the SSR document
	// loads to hydrate.
	let dir = std::path::PathBuf::from(out_base.unwrap_or_else(|| "out".to_string()));
	if let Err(e) = browser_bundle::write_built_dir(&dir, &client_bytes) {
		print_error(format!("writing _built bundle to {}: {e}", dir.display()));
		std::process::exit(1);
	}
	let server_path = dir.join("main.wasm");
	if let Err(e) = std::fs::write(&server_path, &server_bytes) {
		print_error(format!("writing {}: {e}", server_path.display()));
		std::process::exit(1);
	}

	let mut artifacts = vec![
		Artifact::file(&server_path, None),
		// `_built/` holds the client bundle (loader.js + app.wasm) the server serves
		// itself for hydration — written by `write_built_dir` above.
		Artifact {
			path: format!("{}/", dir.join("_built").display()),
			size: "2 files".to_string(),
			note: Some("served for hydration".to_string()),
			files: 2,
		},
	];

	// Carry the app's static assets into the build: `<entry>/public` → `out/public`,
	// so the running server serves `/logo.svg` and friends from the same `public/` it
	// reads under `pluma dev` (`pluma run` chdirs into the bundle dir to find it).
	artifacts.extend(copy_public_assets(&entry_path, &dir));

	print_build_summary(
		&format!("fullstack app → {}/", dir.display()),
		&artifacts,
		&[(
			// The server is the whole app: it SSRs each page, answers `/_rpc/*`, and
			// serves `/_built/*` for hydration — a static file server would skip all of
			// that, so this is the one way to run a fullstack build. `pluma run` chdirs
			// into the wasm's own directory, so it finds its sibling `_built/`, `public/`,
			// and data files no matter where the command is invoked from.
			format!("pluma run {}", server_path.display()),
			"serve page + RPC + client bundle",
		)],
		start.elapsed(),
	);
}

/// Copy `<entry>/public` into the output directory as `public/`, if it exists. The
/// served files live next to the server artifact so a deployment that runs from the
/// output directory finds them at the same relative path the dev server does. A
/// missing `public/` is fine — the app just serves no assets. Returns a summary row
/// when assets were copied (`None` when there's no `public/` dir).
fn copy_public_assets(entry_path: &str, out_dir: &std::path::Path) -> Option<Artifact> {
	let src = std::path::Path::new(entry_path).join("public");
	if !src.is_dir() {
		return None;
	}
	let dst = out_dir.join("public");
	match copy_dir_all(&src, &dst) {
		Ok(n) => Some(Artifact {
			path: format!("{}/", dst.display()),
			size: format!("{n} asset{}", if n == 1 { "" } else { "s" }),
			note: None,
			files: n,
		}),
		Err(e) => {
			print_error(format!(
				"copying {} to {}: {e}",
				src.display(),
				dst.display()
			));
			None
		}
	}
}

/// Recursively copy `src` into `dst`, creating directories as needed. Returns the
/// number of files copied.
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<usize> {
	std::fs::create_dir_all(dst)?;
	let mut count = 0;
	for entry in std::fs::read_dir(src)? {
		let entry = entry?;
		let from = entry.path();
		let to = dst.join(entry.file_name());
		if entry.file_type()?.is_dir() {
			count += copy_dir_all(&from, &to)?;
		} else {
			std::fs::copy(&from, &to)?;
			count += 1;
		}
	}
	Ok(count)
}
