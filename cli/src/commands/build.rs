use compiler::*;

use crate::browser_bundle;
use crate::printing::*;

/// `pluma build [--web] <file> [-o out]` — compile a module to a deploy artifact.
/// By default this lowers the shared IR for a machine/OS host through the WasmGC
/// backend and writes `<out>.wasm`, run with `pluma run <out>.wasm`. `--web` instead
/// lowers for the web/DOM sandbox and writes a browser bundle.
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
			"`--target` was removed. Use `--web` for a browser build; omit it for a server/CLI build.",
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

	// FULLSTACK: a directory with both `main.pa` and `client.pa` builds two
	// artifacts from one source — `main.wasm` + a browser client bundle —
	// regardless of `--web` (each half has its own).
	if Compiler::is_fullstack_dir(&entry_path) {
		build_fullstack(entry_path, out_base, server_url, opt_level, start);
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
		if print_diagnostics_is_fatal(diagnostics) {
			std::process::exit(1);
		}
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
	let bytes = run_wasm_opt(bytes, opt_level);
	match target {
		Target::Sys => {
			let wasm_path = std::path::PathBuf::from(format!("{base}.wasm"));
			if let Err(e) = std::fs::write(&wasm_path, &bytes) {
				print_error(format!("writing {}: {e}", wasm_path.display()));
				std::process::exit(1);
			}
			print_build_summary(
				&wasm_path.display().to_string(),
				&[Artifact::file(&wasm_path, None)],
				&[(format!("pluma run {}", wasm_path.display()), "run it")],
				start.elapsed(),
			);
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
			print_build_summary(
				&format!("web bundle → {}/", dir.display()),
				&[
					Artifact::file(&dir.join("app.wasm"), None),
					Artifact::file(&dir.join("loader.js"), None),
					Artifact::file(&dir.join("index.html"), None),
				],
				&[(
					format!("python3 -m http.server --directory {}", dir.display()),
					"serve it, then open the printed URL",
				)],
				start.elapsed(),
			);
		}
	}
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

	// Artifacts land in an output directory (default `out/`): the server wasm
	// alongside the browser bundle (app.wasm + loader.js + index.html).
	let dir = std::path::PathBuf::from(out_base.unwrap_or_else(|| "out".to_string()));
	if let Err(e) = browser_bundle::write_bundle(&dir, &client_bytes) {
		print_error(format!("writing web bundle to {}: {e}", dir.display()));
		std::process::exit(1);
	}
	// The fullstack server serves the hydration bundle itself, from `_built/`.
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
		Artifact::file(&dir.join("app.wasm"), None),
		Artifact::file(&dir.join("loader.js"), None),
		Artifact::file(&dir.join("index.html"), None),
		// `_built/` holds the client bundle (loader.js + app.wasm) the server serves
		// itself for hydration — written by `write_built_dir` above.
		Artifact {
			path: format!("{}/", dir.join("_built").display()),
			size: "2 files".to_string(),
			note: Some("served for hydration".to_string()),
			files: 2,
		},
	];

	// Build-time CSS extraction: lift the SSR stylesheet into a cacheable `app.css`
	// linked from the static client shell.
	artifacts.extend(extract_css_to_bundle(&dir, &server_path));

	// Carry the app's static assets into the build: `<entry>/public` → `out/public`,
	// so `pluma run out/main.wasm` (started from `out/`) serves `/logo.svg` and
	// friends from the same `public/` the server reads under `pluma dev`.
	artifacts.extend(copy_public_assets(&entry_path, &dir));

	print_build_summary(
		&format!("fullstack app → {}/", dir.display()),
		&artifacts,
		&[(
			// The server is the whole app: it SSRs each page, answers `/_rpc/*`, and
			// serves `/_built/*` for hydration — a static file server would skip all of
			// that, so this is the one way to run a fullstack build. It reads its sibling
			// `_built/`, `public/`, and data files relative to the working directory, so
			// it has to run from inside the output dir — `cd` in, then run `main.wasm`.
			format!("cd {} && pluma run main.wasm", dir.display()),
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

/// Best-effort: run the freshly-built server, `GET /`, and lift the inline `<style>`
/// the server-side render emits (via `css.style-tag`) into a cacheable `app.css`
/// linked from the client `index.html`. The SSR document still inlines its CSS at
/// runtime; `app.css` is for the *static* CSR path — `index.html` served without the
/// SSR server in front of it. Never fails the build: any hiccup (server doesn't SSR
/// `/`, port busy, sandbox blocks the socket) silently skips, leaving the inline-only
/// behaviour unchanged.
fn extract_css_to_bundle(dir: &std::path::Path, server_wasm: &std::path::Path) -> Option<Artifact> {
	let exe = std::env::current_exe().ok()?;
	// Pin the server to a free port via `$PORT` (which `http.serve` honors), so the
	// probe always knows where to reach it — no matter what address `main.pa` names.
	let port = crate::commands::dev::pick_free_port();
	// Run the server as a child (`pluma run main.wasm`), silenced.
	let Ok(mut child) = std::process::Command::new(&exe)
		.arg("run")
		.arg(server_wasm)
		.env("PORT", port.to_string())
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.spawn()
	else {
		return None;
	};
	let doc = fetch_ssr_document(&mut child, port);
	let _ = child.kill();
	let _ = child.wait();

	let css = doc
		.as_deref()
		.and_then(extract_style_block)
		.unwrap_or_default();
	if css.trim().is_empty() {
		// No SSR stylesheet (the app uses no extracted rules, or the server doesn't
		// SSR `/`) — leave the bundle as-is.
		return None;
	}
	let css_path = dir.join("app.css");
	if std::fs::write(&css_path, css.as_bytes()).is_err() {
		return None;
	}
	link_stylesheet_in_index(dir);
	Some(Artifact::file(
		&css_path,
		Some("extracted from SSR".to_string()),
	))
}

/// Connect to the running server on the probe port and `GET /`, returning the HTML
/// body of a 200 response. Retries briefly while the child boots; `None` if it never
/// answers a 200 (e.g. a bare RPC server 404s `/`) or the child exits first (a port
/// clash — fail fast rather than burn the whole retry budget).
fn fetch_ssr_document(child: &mut std::process::Child, port: u16) -> Option<String> {
	use std::io::{Read, Write};
	use std::net::TcpStream;

	for _ in 0..30 {
		std::thread::sleep(std::time::Duration::from_millis(100));
		// If the server already exited (couldn't bind the port), there's nothing to hit.
		if matches!(child.try_wait(), Ok(Some(_))) {
			return None;
		}
		let Ok(mut up) = TcpStream::connect(("127.0.0.1", port)) else {
			continue;
		};
		let req = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
		if up.write_all(req.as_bytes()).is_err() {
			continue;
		}
		let _ = up.flush();
		let mut raw = Vec::new();
		if up.read_to_end(&mut raw).is_err() {
			continue;
		}
		let text = String::from_utf8_lossy(&raw);
		let status_ok = text
			.lines()
			.next()
			.map(|l| l.contains(" 200 "))
			.unwrap_or(false);
		if !status_ok {
			return None;
		}
		return text
			.split_once("\r\n\r\n")
			.map(|(_, body)| body.to_string());
	}
	None
}

/// Lift the concatenated contents of every `<style>…</style>` block out of an HTML
/// document. We emit one (from `css.style-tag`), but join any others defensively.
fn extract_style_block(html: &str) -> Option<String> {
	let mut out = String::new();
	let mut rest = html;
	while let Some(open) = rest.find("<style>") {
		let after = &rest[open + "<style>".len()..];
		let Some(close) = after.find("</style>") else {
			break;
		};
		out.push_str(&after[..close]);
		rest = &after[close + "</style>".len()..];
	}
	if out.is_empty() { None } else { Some(out) }
}

/// Add `<link rel="stylesheet" href="app.css">` to the client `index.html` `<head>`
/// (idempotent). Best-effort — a missing/unreadable file just leaves it unlinked.
fn link_stylesheet_in_index(dir: &std::path::Path) {
	let index = dir.join("index.html");
	let Ok(html) = std::fs::read_to_string(&index) else {
		return;
	};
	if html.contains("href=\"app.css\"") {
		return;
	}
	let linked = html.replacen(
		"</head>",
		"<link rel=\"stylesheet\" href=\"app.css\"></head>",
		1,
	);
	let _ = std::fs::write(&index, linked);
}
