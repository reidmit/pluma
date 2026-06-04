// `pluma dev <path>` — the watch + live-reload loop (Tier 1: full reload).
//
// Two modes, mirroring `pluma build`'s `--target`:
//   - `--target web`  → build the browser bundle, serve it over a built-in HTTP
//     server (WasmGC needs a real origin, not file://), watch `*.pa` sources, and
//     push a reload over Server-Sent Events on every successful rebuild. The page
//     does a full `location.reload()` — state is lost (HMR is a later tier).
//   - `--target sys` (default) → run the program as a child `pluma run` subprocess
//     and restart it whenever a source file changes (classic nodemon-style).
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
use std::time::{Duration, SystemTime};

use compiler::*;

use crate::printing::*;

/// The latest-served wasm bytes (swapped in on each successful rebuild) and the
/// set of connected SSE clients to nudge.
type Served = Arc<Mutex<Vec<u8>>>;
type Clients = Arc<Mutex<Vec<TcpStream>>>;

const POLL: Duration = Duration::from_millis(250);

pub(crate) fn dev_command(args: Vec<String>) {
	let mut entry_path: Option<String> = None;
	let mut target = String::from("sys");
	let mut port: u16 = 2222;
	let mut server_url: Option<String> = None;
	let mut iter = args.into_iter();
	while let Some(a) = iter.next() {
		match a.as_str() {
			"--target" => {
				if let Some(t) = iter.next() {
					target = t;
				}
			}
			"--port" => match iter.next().and_then(|p| p.parse::<u16>().ok()) {
				Some(p) => port = p,
				None => {
					print_error("`--port` requires a number");
					std::process::exit(1);
				}
			},
			"--server-url" => server_url = iter.next(),
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

	// A fullstack directory (`server.pa` + `client.pa`) runs both halves: the server
	// as a subprocess, the client served + live-reloaded, with `/rpc/*` proxied to
	// the server (same origin, so no CORS). The client posts to the dev origin by
	// default (the proxy forwards it); `--server-url` overrides for an external server.
	if Compiler::is_fullstack_dir(&entry_path) {
		let base = server_url.unwrap_or_else(|| format!("http://localhost:{port}"));
		dev_fullstack(entry_path, port, base);
		return;
	}

	match target.as_str() {
		"web" => dev_web(entry_path, port),
		"sys" => dev_server(entry_path),
		other => {
			print_error(format!(
				"Unknown --target `{other}`. Expected `sys` or `web`."
			));
			std::process::exit(1);
		}
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
			Ok(w) => {
				eprintln!("[pluma dev] note: model isn't serializable — HMR off, using full reload");
				(w, false)
			}
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

	println!("pluma dev — serving {entry_path}");
	println!("  http://localhost:{port}/  (live-reload on save)");
	if hmr_on {
		println!("  hot-reload: model state is preserved across edits");
	}
	println!("  ctrl-c to stop");

	// Watch + rebuild loop on the main thread, in the mode chosen at startup.
	let root = watch_root(&entry_path);
	let mut last = scan(&root);
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
				println!("[pluma dev] rebuilt — reloaded {n} client(s)");
			}
			Err(diags) => {
				// Keep serving the last good build while the source is broken.
				print_diagnostics(diags);
				eprintln!("[pluma dev] build failed — keeping previous version");
			}
		}
	}
}

/// Compile `entry_path` for the web target (with `hmr` redirection on or off),
/// returning the wasm bytes or the analysis diagnostics. Post-analysis lower/codegen
/// failures are fatal and unrelated to HMR, so they print and return empty diags.
fn build_web(entry_path: &str, hmr: bool) -> Result<Vec<u8>, Vec<Diagnostic>> {
	let mut compiler = match Compiler::from_entry_path(entry_path.to_string()) {
		Ok(c) => c.with_target(Some(Target::Web)).with_hmr(hmr),
		Err(diagnostics) => return Err(diagnostics),
	};
	if let Err(diagnostics) = compiler.check() {
		return Err(diagnostics);
	}
	let program = match ir::lower(&compiler) {
		Ok(p) => p,
		Err(msg) => {
			print_error(format!("ir::lower: {msg}"));
			return Err(Vec::new());
		}
	};
	match wasm::emit_with_options(
		&program,
		wasm::EmitOptions {
			browser: true,
			..Default::default()
		},
	) {
		Ok(b) => Ok(b),
		Err(diags) => {
			print_error(format!("wasm codegen error: {}", diags.0.join("; ")));
			Err(Vec::new())
		}
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
/// web and fullstack dev servers (the fullstack one handles `/rpc/*` first).
fn serve_static(path: &str, mut stream: TcpStream, served: &Served, clients: &Clients) {
	match path {
		"/" | "/index.html" => respond(
			&mut stream,
			"200 OK",
			"text/html; charset=utf-8",
			INDEX_HTML_DEV.as_bytes(),
		),
		"/loader.js" => respond(
			&mut stream,
			"200 OK",
			"text/javascript; charset=utf-8",
			crate::browser_bundle::LOADER_JS.as_bytes(),
		),
		"/app.wasm" => {
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

// --------------------------------------------------------------------------
// Fullstack mode: run the server subprocess + serve the client, proxying RPC.
// --------------------------------------------------------------------------

// The port the server subprocess binds (the example's `server.pa` uses 8080).
const FULLSTACK_SERVER_PORT: u16 = 8080;

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

	// The server runs as a `pluma run <server.wasm>` child, rebuilt + restarted on
	// change. Keep the wasm in a temp file the child re-reads.
	let server_path = std::env::temp_dir().join("pluma-dev-server.wasm");
	if let Err(e) = std::fs::write(&server_path, &server_bytes) {
		print_error(format!("writing {}: {e}", server_path.display()));
		std::process::exit(1);
	}
	let mut child = spawn_run(&exe, &server_path.to_string_lossy());

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
				thread::spawn(move || handle_conn_fs(stream, served, clients));
			}
		});
	}

	println!("pluma dev — fullstack {entry_path}");
	println!("  http://localhost:{port}/  (client, live-reload on save)");
	println!("  /rpc/* → 127.0.0.1:{FULLSTACK_SERVER_PORT}  (server subprocess)");
	println!("  ctrl-c to stop");

	let root = watch_root(&entry_path);
	let mut last = scan(&root);
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
				child = spawn_run(&exe, &server_path.to_string_lossy());
				*served.lock().unwrap() = client_bytes;
				let n = broadcast_reload(&clients);
				println!("[pluma dev] rebuilt both halves — reloaded {n} client(s)");
			}
			Err(diags) => {
				print_diagnostics(diags);
				eprintln!("[pluma dev] build failed — keeping previous version");
			}
		}
	}
}

/// Compile a fullstack directory to its two artifacts (server wasm, client web
/// bundle wasm), or return the analysis diagnostics. Post-analysis lower/codegen
/// failures print and return empty diags (fatal, unrelated to source typos).
fn build_fullstack_artifacts(
	entry_path: &str,
	server_url: &str,
) -> Result<(Vec<u8>, Vec<u8>), Vec<Diagnostic>> {
	let mut compiler =
		Compiler::from_fullstack_dir(entry_path.to_string())?.with_rpc_base_url(server_url.to_string());
	compiler.check()?;
	compiler.gate_fullstack()?;
	let server = compiler.entry_modules[0].clone();
	let client = compiler.entry_modules[1].clone();
	let emit = |entry: &str, browser: bool| -> Result<Vec<u8>, Vec<Diagnostic>> {
		let program = ir::lower_entry(&compiler, entry).map_err(|msg| {
			print_error(format!("ir::lower: {msg}"));
			Vec::new()
		})?;
		wasm::emit_with_options(
			&program,
			wasm::EmitOptions {
				browser,
				..Default::default()
			},
		)
		.map_err(|diags| {
			print_error(format!("wasm codegen error: {}", diags.0.join("; ")));
			Vec::new()
		})
	};
	let server_bytes = emit(&server, false)?;
	let client_bytes = emit(&client, true)?;
	Ok((server_bytes, client_bytes))
}

/// Like `handle_conn`, but proxies `/rpc/*` to the server subprocess (same origin,
/// so the browser needs no CORS) and serves the client bundle for everything else.
fn handle_conn_fs(stream: TcpStream, served: Served, clients: Clients) {
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
	if path.starts_with("/rpc/") {
		let mut body = vec![0u8; content_length];
		if content_length > 0 && reader.read_exact(&mut body).is_err() {
			respond(&mut stream, "400 Bad Request", "text/plain", b"short body");
			return;
		}
		proxy_rpc(&mut stream, &request_line, &headers, &body);
		return;
	}
	serve_static(&path, stream, &served, &clients);
}

/// Forward one RPC request verbatim to the server subprocess and relay its response
/// back. The server speaks `Connection: close`, so reading to EOF gets the whole
/// reply. A down server surfaces as a 502 (the client stub turns it into a clean
/// transport failure).
fn proxy_rpc(downstream: &mut TcpStream, request_line: &str, headers: &[String], body: &[u8]) {
	let mut up = match TcpStream::connect(("127.0.0.1", FULLSTACK_SERVER_PORT)) {
		Ok(u) => u,
		Err(_) => {
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
	for h in headers {
		req.extend_from_slice(h.as_bytes());
	}
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
	let mut resp = Vec::new();
	let _ = up.read_to_end(&mut resp);
	let _ = downstream.write_all(&resp);
	let _ = downstream.flush();
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

	println!("pluma dev — running {entry_path} (restart on save, ctrl-c to stop)");
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
		println!("\n[pluma dev] change detected — restarting");
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

/// A cheap change fingerprint: (count of `*.pa` files, latest mtime among them).
/// Comparing this across polls catches edits, additions, and deletions. Hidden
/// directories (`.git`, `target`, …) are skipped.
fn scan(root: &Path) -> (usize, Option<SystemTime>) {
	fn walk(dir: &Path, count: &mut usize, latest: &mut Option<SystemTime>) {
		let entries = match std::fs::read_dir(dir) {
			Ok(e) => e,
			Err(_) => return,
		};
		for entry in entries.flatten() {
			let name = entry.file_name();
			let name = name.to_string_lossy();
			if name.starts_with('.') || name == "target" {
				continue;
			}
			let file_type = match entry.file_type() {
				Ok(t) => t,
				Err(_) => continue,
			};
			let path = entry.path();
			if file_type.is_dir() {
				walk(&path, count, latest);
			} else if name.ends_with(".pa") {
				*count += 1;
				if let Ok(m) = entry.metadata().and_then(|m| m.modified()) {
					if latest.map_or(true, |b| m > b) {
						*latest = Some(m);
					}
				}
			}
		}
	}
	let mut count = 0;
	let mut latest = None;
	walk(root, &mut count, &mut latest);
	(count, latest)
}
