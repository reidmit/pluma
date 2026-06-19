// `pluma dev <path>` — the watch + live-reload loop (Tier 1: full reload).
//
// Two modes, mirroring `pluma build`'s `--web`:
//   - `--web`  → build the browser bundle, serve it over a built-in HTTP
//     server (WasmGC needs a real origin, not file://), watch `*.pa` sources, and
//     push a reload over Server-Sent Events on every successful rebuild. The page
//     does a full `location.reload()` — state is lost (HMR is a later tier).
//   - default → run the program as a child `pluma run` subprocess and restart it
//     whenever a source file changes (classic nodemon-style).
//
// No external dependencies: a hand-rolled HTTP/1.1 + SSE server over `std::net`,
// and a mtime-polling watcher. A full rebuild is ~tens of ms (the whole pipeline
// runs in-process), so polling at a quarter-second is plenty responsive.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::colors::Style;
use crate::watch::scan;

use compiler::*;

use crate::printing::*;

/// The latest-served wasm bytes (swapped in on each successful rebuild) and the
/// set of connected SSE clients to nudge.
type Served = Arc<Mutex<Vec<u8>>>;
type Clients = Arc<Mutex<Vec<TcpStream>>>;

const POLL: Duration = Duration::from_millis(250);

// How long a proxy/SSR request will wait for the server subprocess to come back
// when it can't connect — long enough to bridge a rebuild's kill→respawn→bind gap
// (typically well under a second) without hanging forever on a genuinely dead
// server. Normal traffic connects on the first try and never waits.
const RESTART_GRACE: Duration = Duration::from_secs(5);

// --------------------------------------------------------------------------
// The dev dashboard: a full-screen status panel for the browser-facing modes
// (web + fullstack), redrawn on every rebuild. The app runs in the browser, so
// the terminal is free for a live status display:
//
//     pluma dev · fullstack
//
//     ▸ examples/tasks/
//
//     ● ready
//       build #3 · reloaded 1 client
//
//     http://localhost:2222/    client · live-reload
//     /_rpc/*  →  127.0.0.1:8080  server subprocess
//
//     ctrl-c to stop
//
// On a failed rebuild the panel goes red and renders the compiler diagnostics
// inline (the last good build keeps serving); fix the error and it flips green.
// When stdout can't take ANSI (not a TTY, or NO_COLOR) `tui` is false and the
// dashboard degrades to the old one-line-per-event logging, so piped output and
// CI stay readable.

/// The resting state of the panel: ready (with an optional last-build detail), or
/// failing (carrying the rendered diagnostics to show inline).
enum Status {
	Ready(Option<String>),
	Failed(String),
}

struct Dashboard {
	tui: bool,
	style: Style,
	mode: &'static str,
	entry: String,
	/// A persistent sub-line under `ready` (e.g. the HMR note). `None` to omit.
	note: Option<String>,
	/// `(target, annotation)` rows — the served URL and the RPC proxy. The target
	/// is painted cyan, the annotation dim.
	rows: Vec<(String, String)>,
}

impl Dashboard {
	fn new(
		mode: &'static str,
		entry: String,
		note: Option<String>,
		rows: Vec<(String, String)>,
	) -> Self {
		let on = crate::colors::should_colorize();
		Dashboard {
			tui: on,
			style: Style { on },
			mode,
			entry,
			note,
			rows,
		}
	}

	fn draw(&self, status: &Status) {
		if !self.tui {
			self.draw_plain(status);
			return;
		}
		let s = self.style;
		let mut o = String::new();
		// Clear the visible screen and home the cursor (watch-style; scrollback is
		// left intact so the user's prior history isn't destroyed).
		o.push_str("\x1b[2J\x1b[H\n");
		o += &format!(
			"  {} {}\n\n",
			s.bold("pluma dev"),
			s.dim(&format!("· {}", self.mode))
		);
		o += &format!("  {} {}\n\n", s.dim("▸"), self.entry);

		match status {
			Status::Ready(last) => {
				o += &format!("  {} {}\n", s.green("●"), s.bold("ready"));
				if let Some(n) = &self.note {
					o += &format!("    {}\n", s.dim(n));
				}
				if let Some(l) = last {
					o += &format!("    {}\n", s.dim(l));
				}
			}
			Status::Failed(diags) => {
				o += &format!("  {} {}\n\n", s.red("●"), s.bold("build failed"));
				// The diagnostics are already colorized; just indent them into the panel.
				for line in diags.lines() {
					o += &format!("  {line}\n");
				}
				o += &format!(
					"\n  {}\n",
					s.yellow("serving last good build — fix the error to reload")
				);
			}
		}
		o.push('\n');

		// Align the dim annotations: pad each target to the widest one.
		let w = self
			.rows
			.iter()
			.map(|(l, _)| l.chars().count())
			.max()
			.unwrap_or(0);
		for (target, annotation) in &self.rows {
			let pad = " ".repeat(w.saturating_sub(target.chars().count()));
			o += &format!("  {}{}   {}\n", s.cyan(target), pad, s.dim(annotation));
		}
		if !self.rows.is_empty() {
			o.push('\n');
		}
		o += &format!("  {}\n", s.dim("ctrl-c to stop"));

		print!("{o}");
		let _ = std::io::stdout().flush();
	}

	/// Non-TTY fallback: a banner once at startup, then one line per event — the
	/// behavior `pluma dev` had before the dashboard.
	fn draw_plain(&self, status: &Status) {
		match status {
			Status::Ready(None) => {
				println!("pluma dev — {} {}", self.mode, self.entry);
				for (target, annotation) in &self.rows {
					println!("  {target}  ({annotation})");
				}
				if let Some(n) = &self.note {
					println!("  {n}");
				}
				println!("  ctrl-c to stop");
			}
			Status::Ready(Some(detail)) => println!("[pluma dev] {detail}"),
			Status::Failed(diags) => {
				eprint!("{diags}");
				eprintln!("[pluma dev] build failed — keeping previous version");
			}
		}
	}
}

pub(crate) fn dev_command(web: bool, port: u16, server_url: Option<String>, entry_path: String) {
	// A fullstack directory (`server.pa` + `client.pa`) runs both halves: the server
	// as a subprocess, the client served + live-reloaded, with `/_rpc/*` proxied to
	// the server (same origin, so no CORS). The client posts same-origin by default
	// (a relative `/_rpc/...`, which the proxy forwards regardless of how the page was
	// reached — localhost vs 127.0.0.1); `--server-url` overrides for an external server.
	if Compiler::is_fullstack_dir(&entry_path) {
		let base = server_url.unwrap_or_default();
		dev_fullstack(entry_path, port, base);
		return;
	}

	if web {
		dev_web(entry_path, port);
	} else {
		dev_server(entry_path);
	}
}

// --------------------------------------------------------------------------
// Browser mode: serve the bundle + live-reload over SSE.
// --------------------------------------------------------------------------

fn dev_web(entry_path: String, port: u16) {
	// Try the model-preserving HMR build first; if the model isn't `wire`-able the
	// analyzer rejects the `-hmr` redirect, so fall back to a plain (full-reload)
	// build. We must start from a compiling state — there's nothing to serve
	// otherwise. `hmr_on` is decided once here and held for the session.
	let (wasm, hmr_on) = match build_web(&entry_path, true) {
		Ok(w) => (w, true),
		Err(_) => match build_web(&entry_path, false) {
			Ok(w) => (w, false),
			Err(diags) => {
				print_diagnostics(diags);
				std::process::exit(1);
			}
		},
	};

	let served: Served = Arc::new(Mutex::new(wasm));
	let clients: Clients = Arc::new(Mutex::new(Vec::new()));

	let listener = match TcpListener::bind(("127.0.0.1", port)) {
		Ok(l) => l,
		Err(e) => {
			print_error(format!(
				"could not bind 127.0.0.1:{port}: {e} (is another `pluma dev` running? try --port)"
			));
			std::process::exit(1);
		}
	};

	// Accept loop on its own thread; a thread per connection (SSE streams stay open).
	{
		let served = served.clone();
		let clients = clients.clone();
		thread::spawn(move || {
			for stream in listener.incoming().flatten() {
				let served = served.clone();
				let clients = clients.clone();
				thread::spawn(move || handle_conn(stream, served, clients));
			}
		});
	}

	let note = Some(if hmr_on {
		"hot-reload — model state is preserved across edits".to_string()
	} else {
		"full reload — model isn't serializable, so HMR is off".to_string()
	});
	let dash = Dashboard::new(
		"web",
		entry_path.clone(),
		note,
		vec![(
			format!("http://localhost:{port}/"),
			"live-reload on save".to_string(),
		)],
	);
	dash.draw(&Status::Ready(None));

	// Watch + rebuild loop on the main thread, in the mode chosen at startup.
	let root = watch_root(&entry_path);
	let mut last = scan(&root);
	let mut builds = 0usize;
	loop {
		thread::sleep(POLL);
		let now = scan(&root);
		if now == last {
			continue;
		}
		last = now;
		match build_web(&entry_path, hmr_on) {
			Ok(w) => {
				*served.lock().unwrap() = w;
				let n = broadcast_reload(&clients);
				builds += 1;
				dash.draw(&Status::Ready(Some(format!(
					"build #{builds} · reloaded {n} client(s)"
				))));
			}
			Err(diags) => {
				// Keep serving the last good build while the source is broken.
				dash.draw(&Status::Failed(render_diagnostics_string(&diags)));
			}
		}
	}
}

/// Compile `entry_path` for the web target (with `hmr` redirection on or off),
/// returning the wasm bytes or the diagnostics to display. Post-analysis lower/
/// codegen failures are reported as a synthetic `Diagnostic` so they surface in
/// the dashboard alongside ordinary type errors.
fn build_web(entry_path: &str, hmr: bool) -> Result<Vec<u8>, Vec<Diagnostic>> {
	let mut compiler = match Compiler::from_entry_path(entry_path.to_string()) {
		Ok(c) => c.with_target(Some(Target::Web)).with_hmr(hmr),
		Err(diagnostics) => return Err(diagnostics),
	};
	if let Err(diagnostics) = compiler.check() {
		// Errors fail the build (red panel); warning-only diagnostics don't block dev.
		if diagnostics.iter().any(Diagnostic::is_error) {
			return Err(diagnostics);
		}
	}
	let program = match ir::lower(&compiler) {
		Ok(p) => p,
		Err(msg) => return Err(vec![Diagnostic::error(format!("ir::lower: {msg}"))]),
	};
	match wasm::emit_with_options(
		&program,
		wasm::EmitOptions {
			browser: true,
			..Default::default()
		},
	) {
		Ok(b) => Ok(b),
		Err(diags) => Err(vec![Diagnostic::error(format!(
			"wasm codegen error: {}",
			diags.0.join("; ")
		))]),
	}
}

/// Serve one HTTP request. Routes: the dev HTML shell, the loader, the live wasm,
/// and the SSE reload channel (which keeps its stream open, parked in `clients`).
fn handle_conn(stream: TcpStream, served: Served, clients: Clients) {
	let clone = match stream.try_clone() {
		Ok(s) => s,
		Err(_) => return,
	};
	let mut reader = BufReader::new(clone);
	let mut request_line = String::new();
	if reader.read_line(&mut request_line).is_err() {
		return;
	}
	// Drain the rest of the request headers (we don't need them).
	loop {
		let mut h = String::new();
		match reader.read_line(&mut h) {
			Ok(0) => break,
			Ok(_) if h == "\r\n" || h == "\n" => break,
			Ok(_) => {}
			Err(_) => break,
		}
	}

	let path = request_line.split_whitespace().nth(1).unwrap_or("/");
	serve_static(path, stream, &served, &clients);
}

/// Serve the dev shell / loader / live wasm / SSE channel for `path`. Shared by the
/// web and fullstack dev servers (the fullstack one handles `/_rpc/*` first).
fn serve_static(path: &str, mut stream: TcpStream, served: &Served, clients: &Clients) {
	match path {
		"/" | "/index.html" => respond(
			&mut stream,
			"200 OK",
			"text/html; charset=utf-8",
			INDEX_HTML_DEV.as_bytes(),
		),
		// The hydration bundle is served at the root (`pluma dev --web` shell) and at
		// the reserved `/_built/` prefix (what a fullstack SSR document references).
		"/loader.js" | "/_built/loader.js" => respond(
			&mut stream,
			"200 OK",
			"text/javascript; charset=utf-8",
			crate::browser_bundle::LOADER_JS.as_bytes(),
		),
		"/app.wasm" | "/_built/app.wasm" => {
			let bytes = served.lock().unwrap().clone();
			respond(&mut stream, "200 OK", "application/wasm", &bytes);
		}
		"/__livereload" => {
			let head = "HTTP/1.1 200 OK\r\n\
				Content-Type: text/event-stream\r\n\
				Cache-Control: no-store\r\n\
				Connection: keep-alive\r\n\r\n";
			if stream.write_all(head.as_bytes()).is_ok()
				&& stream.write_all(b": connected\n\n").is_ok()
				&& stream.flush().is_ok()
			{
				// Park the open stream; the watch loop writes reload events to it.
				clients.lock().unwrap().push(stream);
			}
		}
		_ => respond(&mut stream, "404 Not Found", "text/plain", b"not found"),
	}
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
	let head = format!(
		"HTTP/1.1 {status}\r\n\
		Content-Type: {content_type}\r\n\
		Content-Length: {}\r\n\
		Cache-Control: no-store\r\n\
		Connection: close\r\n\r\n",
		body.len()
	);
	let _ = stream.write_all(head.as_bytes());
	let _ = stream.write_all(body);
	let _ = stream.flush();
}

/// Push a reload event to every connected SSE client, dropping any that have
/// disconnected. Returns how many are still live.
fn broadcast_reload(clients: &Clients) -> usize {
	let mut guard = clients.lock().unwrap();
	guard.retain_mut(|c| {
		c.write_all(b"data: reload\n\n")
			.and_then(|_| c.flush())
			.is_ok()
	});
	guard.len()
}

// The HTML shell served in dev: the same `loader.js` the build bundle uses, plus a
// tiny EventSource client that reloads on the server's `reload` event.
const INDEX_HTML_DEV: &str = "<!doctype html>\n\
<html>\n\
<head><meta charset=\"utf-8\"><title>Pluma dev</title></head>\n\
<body>\n\
<script type=\"module\" src=\"loader.js\"></script>\n\
<script>\n\
const es = new EventSource('/__livereload');\n\
es.onmessage = (e) => { if (e.data === 'reload') location.reload(); };\n\
es.onerror = () => {};\n\
</script>\n\
</body>\n\
</html>\n";

// The live-reload EventSource client, injected before `</body>` of a server-rendered
// page (the same snippet baked into `INDEX_HTML_DEV` for the non-SSR shell).
const LIVERELOAD_SNIPPET: &str = "<script>\n\
const es = new EventSource('/__livereload');\n\
es.onmessage = (e) => { if (e.data === 'reload') location.reload(); };\n\
es.onerror = () => {};\n\
</script>\n";

// --------------------------------------------------------------------------
// Fullstack mode: run the server subprocess + serve the client, proxying RPC.
// --------------------------------------------------------------------------

fn dev_fullstack(entry_path: String, port: u16, server_url: String) {
	let exe = match std::env::current_exe() {
		Ok(p) => p,
		Err(e) => {
			print_error(format!("could not locate the pluma binary: {e}"));
			std::process::exit(1);
		}
	};
	// Both halves must compile before we serve anything.
	let (server_bytes, client_bytes) = match build_fullstack_artifacts(&entry_path, &server_url) {
		Ok(pair) => pair,
		Err(diags) => {
			print_diagnostics(diags);
			std::process::exit(1);
		}
	};

	// Pick a free port for the server subprocess and hand it over via `$PORT`, which
	// `http.serve` honors — so the server binds wherever we put it instead of the
	// literal address in `server.pa`, and the proxy below always knows where it is.
	// No more guessing a hardcoded 8080 (and no collision when 8080 is taken).
	let server_port = pick_free_port();

	// The server runs as a `pluma run <server.wasm>` child, rebuilt + restarted on
	// change. Keep the wasm in a temp file the child re-reads.
	let server_path = std::env::temp_dir().join("pluma-dev-server.wasm");
	if let Err(e) = std::fs::write(&server_path, &server_bytes) {
		print_error(format!("writing {}: {e}", server_path.display()));
		std::process::exit(1);
	}
	let mut child = spawn_server(
		&exe,
		&server_path.to_string_lossy(),
		server_port,
		&entry_path,
	);

	let served: Served = Arc::new(Mutex::new(client_bytes));
	let clients: Clients = Arc::new(Mutex::new(Vec::new()));

	let listener = match TcpListener::bind(("127.0.0.1", port)) {
		Ok(l) => l,
		Err(e) => {
			print_error(format!(
				"could not bind 127.0.0.1:{port}: {e} (is another `pluma dev` running? try --port)"
			));
			std::process::exit(1);
		}
	};
	{
		let served = served.clone();
		let clients = clients.clone();
		thread::spawn(move || {
			for stream in listener.incoming().flatten() {
				let served = served.clone();
				let clients = clients.clone();
				thread::spawn(move || handle_conn_fs(stream, served, clients, server_port));
			}
		});
	}

	let dash = Dashboard::new(
		"fullstack",
		entry_path.clone(),
		None,
		vec![
			(
				format!("http://localhost:{port}/"),
				"client · live-reload".to_string(),
			),
			(
				format!("/_rpc/*  →  127.0.0.1:{server_port}"),
				"server subprocess".to_string(),
			),
		],
	);
	dash.draw(&Status::Ready(None));

	let root = watch_root(&entry_path);
	let mut last = scan(&root);
	let mut builds = 0usize;
	loop {
		thread::sleep(POLL);
		let now = scan(&root);
		if now == last {
			continue;
		}
		last = now;
		match build_fullstack_artifacts(&entry_path, &server_url) {
			Ok((server_bytes, client_bytes)) => {
				// Restart the server with the new artifact, swap the client, reload.
				let _ = child.kill();
				let _ = child.wait();
				if let Err(e) = std::fs::write(&server_path, &server_bytes) {
					print_error(format!("writing {}: {e}", server_path.display()));
				}
				child = spawn_server(
					&exe,
					&server_path.to_string_lossy(),
					server_port,
					&entry_path,
				);
				*served.lock().unwrap() = client_bytes;
				// Wait for the new server to start listening before telling the browser
				// to reload — otherwise the reloaded page's SSR/RPC fetches race the
				// still-binding server and the user sees a 502 flash.
				wait_for_server(server_port);
				let n = broadcast_reload(&clients);
				builds += 1;
				dash.draw(&Status::Ready(Some(format!(
					"build #{builds} · rebuilt both halves · reloaded {n} client(s)"
				))));
			}
			Err(diags) => {
				dash.draw(&Status::Failed(render_diagnostics_string(&diags)));
			}
		}
	}
}

/// Compile a fullstack directory to its two artifacts (server wasm, client web
/// bundle wasm), or return the diagnostics to display. Post-analysis lower/codegen
/// failures are reported as a synthetic `Diagnostic` (so they show in the dashboard).
fn build_fullstack_artifacts(
	entry_path: &str,
	server_url: &str,
) -> Result<(Vec<u8>, Vec<u8>), Vec<Diagnostic>> {
	// Try the model-preserving HMR redirect for the client (`app.element`/
	// `app.application` → their `-dev` variants), so the live model survives a dev
	// reload. If the client's model isn't `wire`-able the analyzer rejects the
	// redirect, so fall back to a plain (full-reload) client. The server build is
	// identical either way; mirrors the single-file `dev_web` hmr-then-plain probe.
	build_fullstack_with_hmr(entry_path, server_url, true)
		.or_else(|_| build_fullstack_with_hmr(entry_path, server_url, false))
}

fn build_fullstack_with_hmr(
	entry_path: &str,
	server_url: &str,
	hmr: bool,
) -> Result<(Vec<u8>, Vec<u8>), Vec<Diagnostic>> {
	let mut compiler = Compiler::from_fullstack_dir(entry_path.to_string())?
		.with_rpc_base_url(server_url.to_string())
		.with_hmr(hmr);
	if let Err(diagnostics) = compiler.check() {
		// Errors fail the build (red panel); warning-only diagnostics don't block dev.
		if diagnostics.iter().any(Diagnostic::is_error) {
			return Err(diagnostics);
		}
	}
	compiler.gate_fullstack()?;
	let server = compiler.entry_modules[0].clone();
	let client = compiler.entry_modules[1].clone();
	let emit = |entry: &str, browser: bool| -> Result<Vec<u8>, Vec<Diagnostic>> {
		let program = ir::lower_entry(&compiler, entry)
			.map_err(|msg| vec![Diagnostic::error(format!("ir::lower: {msg}"))])?;
		wasm::emit_with_options(
			&program,
			wasm::EmitOptions {
				browser,
				..Default::default()
			},
		)
		.map_err(|diags| {
			vec![Diagnostic::error(format!(
				"wasm codegen error: {}",
				diags.0.join("; ")
			))]
		})
	};
	let server_bytes = emit(&server, false)?;
	let client_bytes = emit(&client, true)?;
	Ok((server_bytes, client_bytes))
}

/// Like `handle_conn`, but proxies `/_rpc/*` to the server subprocess (same origin,
/// so the browser needs no CORS) and serves the client bundle for everything else.
fn handle_conn_fs(stream: TcpStream, served: Served, clients: Clients, server_port: u16) {
	let clone = match stream.try_clone() {
		Ok(s) => s,
		Err(_) => return,
	};
	let mut reader = BufReader::new(clone);
	let mut request_line = String::new();
	if reader.read_line(&mut request_line).is_err() {
		return;
	}
	// Collect the raw header lines (kept verbatim for the proxy) + the body length.
	let mut headers: Vec<String> = Vec::new();
	let mut content_length = 0usize;
	loop {
		let mut h = String::new();
		match reader.read_line(&mut h) {
			Ok(0) => break,
			Ok(_) if h == "\r\n" || h == "\n" => break,
			Ok(_) => {
				if let Some(v) = h
					.split_once(':')
					.filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
				{
					content_length = v.1.trim().parse().unwrap_or(0);
				}
				headers.push(h);
			}
			Err(_) => break,
		}
	}

	let path = request_line
		.split_whitespace()
		.nth(1)
		.unwrap_or("/")
		.to_string();
	let mut stream = stream;
	if path.starts_with("/_rpc/") {
		let mut body = vec![0u8; content_length];
		if content_length > 0 && reader.read_exact(&mut body).is_err() {
			respond(&mut stream, "400 Bad Request", "text/plain", b"short body");
			return;
		}
		proxy_rpc(&mut stream, &request_line, &headers, &body, server_port);
		return;
	}
	// Everything that isn't an RPC call or a client-bundle file is answered by the
	// server subprocess at the SAME path the browser asked for, and relayed back:
	//   - an HTML page (the SSR document for `/`, `/reference`, …) gets the live-reload
	//     client injected before the browser hydrates it;
	//   - anything else the server returns 2xx for — a static asset like `/logo.svg`,
	//     a JSON endpoint — is forwarded verbatim, content-type and all, so binary
	//     assets reach the browser intact instead of being mislabelled as HTML.
	// The client bundle's own files are served locally (never treated as page routes).
	// If the server is down or answers non-2xx, fall back to the plain static handler
	// (the CSR dev shell for `/`, else 404).
	let is_asset = matches!(
		path.as_str(),
		"/loader.js" | "/app.wasm" | "/_built/loader.js" | "/_built/app.wasm" | "/__livereload"
	);
	if !is_asset {
		if let Some(resp) = fetch_server_response(server_port, &path) {
			if resp.is_html {
				let body = String::from_utf8_lossy(&resp.body);
				let injected = body.replacen("</body>", &format!("{LIVERELOAD_SNIPPET}</body>"), 1);
				respond(
					&mut stream,
					&resp.status,
					"text/html; charset=utf-8",
					injected.as_bytes(),
				);
				return;
			} else if resp.is_2xx {
				respond(&mut stream, &resp.status, &resp.content_type, &resp.body);
				return;
			}
		}
	}
	serve_static(&path, stream, &served, &clients);
}

/// One response relayed from the server subprocess, kept as raw bytes so binary
/// assets survive the hop. `status` is the reason phrase (`"200 OK"`), `is_html`
/// flags an HTML body (the live-reload inject + UTF-8 path), and `is_2xx` gates
/// whether a non-HTML body is forwarded or the caller falls back to the dev shell.
struct ServerResponse {
	status: String,
	content_type: String,
	body: Vec<u8>,
	is_html: bool,
	is_2xx: bool,
}

/// `GET {path}` against the server subprocess and capture its full response. `None`
/// only when the server can't be reached or its head is unparseable — a reached
/// server's answer (any status) comes back as `Some`, for the caller to relay.
fn fetch_server_response(server_port: u16, path: &str) -> Option<ServerResponse> {
	let mut up = connect_server(server_port)?;
	let req =
		format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{server_port}\r\nConnection: close\r\n\r\n");
	up.write_all(req.as_bytes()).ok()?;
	up.flush().ok()?;
	let mut raw = Vec::new();
	up.read_to_end(&mut raw).ok()?;
	// Split head from body on the blank line; the body stays raw (it may be binary).
	let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
	let body = raw[split + 4..].to_vec();
	let head = String::from_utf8_lossy(&raw[..split]);
	let mut lines = head.lines();
	// "HTTP/1.1 200 OK" → status = "200 OK", code = 200.
	let status = lines.next()?.splitn(2, ' ').nth(1)?.trim().to_string();
	let code: u16 = status.split_whitespace().next()?.parse().ok()?;
	let content_type = lines
		.find_map(|l| {
			l.split_once(':')
				.filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-type"))
				.map(|(_, v)| v.trim().to_string())
		})
		.unwrap_or_else(|| "application/octet-stream".to_string());
	Some(ServerResponse {
		is_html: content_type.contains("text/html"),
		is_2xx: (200..300).contains(&code),
		status,
		content_type,
		body,
	})
}

/// Forward one RPC request verbatim to the server subprocess and relay its response
/// back. The relay is *streamed* (`io::copy` in chunks, not read-to-EOF-then-write):
/// a streaming endpoint's response (`Connection: close` SSE that stays open for
/// minutes) must reach the browser frame-by-frame, not all at once — buffering it
/// would hang every subscription forever. A unary reply still works (copy ends at
/// the server's close). A down server surfaces as a 502 (the client stub turns it
/// into a clean transport failure).
fn proxy_rpc(
	downstream: &mut TcpStream,
	request_line: &str,
	headers: &[String],
	body: &[u8],
	server_port: u16,
) {
	let mut up = match connect_server(server_port) {
		Some(u) => u,
		None => {
			respond(
				downstream,
				"502 Bad Gateway",
				"text/plain",
				b"server not running",
			);
			return;
		}
	};
	let mut req = Vec::new();
	req.extend_from_slice(request_line.as_bytes());
	// Forward one request per upstream connection: drop the browser's own
	// `Connection` header and force `Connection: close`. The server speaks HTTP/1.1
	// keep-alive by default, so without this it holds the connection open after a
	// unary reply — the `io::copy` relay below would never see EOF (it would block
	// forever), and the browser, told `keep-alive`, would pool this proxy socket and
	// reuse it for its next navigation, which the stuck relay thread never services.
	// A streaming reply already frames itself by close, so this is a no-op there.
	for h in headers {
		if h
			.split_once(':')
			.is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case("connection"))
		{
			continue;
		}
		req.extend_from_slice(h.as_bytes());
	}
	req.extend_from_slice(b"Connection: close\r\n");
	req.extend_from_slice(b"\r\n");
	req.extend_from_slice(body);
	if up.write_all(&req).and_then(|_| up.flush()).is_err() {
		respond(
			downstream,
			"502 Bad Gateway",
			"text/plain",
			b"server write failed",
		);
		return;
	}
	// Stream upstream→downstream as bytes arrive so SSE frames are relayed live.
	let mut down = match downstream.try_clone() {
		Ok(d) => d,
		Err(_) => return,
	};
	let _ = std::io::copy(&mut up, &mut down);
	let _ = down.flush();
}

// --------------------------------------------------------------------------
// Server mode: restart a `pluma run` child on change.
// --------------------------------------------------------------------------

fn dev_server(entry_path: String) {
	let exe = match std::env::current_exe() {
		Ok(p) => p,
		Err(e) => {
			print_error(format!("could not locate the pluma binary: {e}"));
			std::process::exit(1);
		}
	};

	// The child program owns the terminal (it prints its own output), so server
	// mode keeps the plain line log — just lightly styled — rather than a full
	// dashboard that the child's stdout would fight with.
	let s = Style {
		on: crate::colors::should_colorize(),
	};
	println!(
		"{} {} {}",
		s.bold("pluma dev"),
		s.dim(&format!("· running {entry_path}")),
		s.dim("(restart on save, ctrl-c to stop)")
	);
	let mut child = spawn_run(&exe, &entry_path);

	// On ctrl-c the terminal signals the whole foreground process group, so the
	// child receives SIGINT alongside us and exits on its own — no cleanup needed.
	let root = watch_root(&entry_path);
	let mut last = scan(&root);
	loop {
		thread::sleep(POLL);
		let now = scan(&root);
		if now == last {
			continue;
		}
		last = now;
		println!("\n{}", s.dim("[pluma dev] change detected — restarting"));
		let _ = child.kill();
		let _ = child.wait();
		child = spawn_run(&exe, &entry_path);
	}
}

fn spawn_run(exe: &Path, entry_path: &str) -> Child {
	match Command::new(exe).arg("run").arg(entry_path).spawn() {
		Ok(c) => c,
		Err(e) => {
			print_error(format!("could not start `pluma run {entry_path}`: {e}"));
			std::process::exit(1);
		}
	}
}

/// Like `spawn_run`, but pins the child's listening port via `$PORT` (which
/// `http.serve` honors). The fullstack server subprocess binds there instead of
/// the literal address in `server.pa`, so the dev proxy and the server always
/// agree on the port without hardcoding one.
/// Connect to the server subprocess, retrying briefly so a request that lands
/// mid-rebuild — after the old server is killed but before the new one has bound
/// its port — waits for the restart instead of failing with a 502. Normal traffic
/// connects on the first attempt with no added latency; a server that's genuinely
/// dead surfaces as `None` once `RESTART_GRACE` elapses.
fn connect_server(port: u16) -> Option<TcpStream> {
	let start = Instant::now();
	loop {
		match TcpStream::connect(("127.0.0.1", port)) {
			Ok(s) => return Some(s),
			Err(_) if start.elapsed() < RESTART_GRACE => {
				thread::sleep(Duration::from_millis(25));
			}
			Err(_) => return None,
		}
	}
}

/// Block until the freshly-spawned server subprocess is accepting connections (or
/// `RESTART_GRACE` passes). Gating the live-reload broadcast on this is what keeps
/// the browser from reloading into a not-yet-listening server and flashing an error
/// page: by the time we say "reload", the new server can already answer.
fn wait_for_server(port: u16) -> bool {
	connect_server(port).is_some()
}

fn spawn_server(exe: &Path, server_wasm: &str, port: u16, cwd: &str) -> Child {
	match Command::new(exe)
		.arg("run")
		.arg(server_wasm)
		// Run the server from its own app directory, so relative paths it reads
		// (a `public/` of static assets, a data file) resolve the same way under
		// `pluma dev` as they will in a deployment that runs from the build output.
		.current_dir(cwd)
		.env("PORT", port.to_string())
		.spawn()
	{
		Ok(c) => c,
		Err(e) => {
			print_error(format!("could not start the server subprocess: {e}"));
			std::process::exit(1);
		}
	}
}

/// Grab a currently-free TCP port by binding `:0` (the OS assigns one) and reading
/// it back. There's a small race — another process could claim it between this
/// drop and the child's bind — but on loopback ephemeral ports that's negligible,
/// and it's how dev servers everywhere pick a port. Falls back to 8080 if the bind
/// somehow fails.
pub(crate) fn pick_free_port() -> u16 {
	TcpListener::bind(("127.0.0.1", 0))
		.ok()
		.and_then(|l| l.local_addr().ok())
		.map(|a| a.port())
		.unwrap_or(8080)
}

// --------------------------------------------------------------------------
// Source watching: poll the project tree's `*.pa` files for changes.
// --------------------------------------------------------------------------

/// The directory to watch: the entry's project root (nearest `pluma.pa`), else the
/// directory containing the entry.
fn watch_root(entry_path: &str) -> PathBuf {
	let p = Path::new(entry_path);
	let start = if p.is_dir() {
		p.to_path_buf()
	} else {
		p.parent()
			.map(|d| d.to_path_buf())
			.unwrap_or_else(|| PathBuf::from("."))
	};
	find_project_root(&start).unwrap_or(start)
}
