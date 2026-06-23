// `std/sys/net` — the host-side socket table + I/O reactor: byte-level TCP ops plus a
// `polling` readiness reactor. The in-wasm scheduler owns the loop; when its ready
// queue empties and socket I/O is in flight, it calls the blocking `net-poll` import
// here (the reactor step). The suspending ops (accept/read/write) are
// *non-blocking* host calls: on `WouldBlock` they register the socket's fd under
// the parked fiber's id (token = fid) and signal would-block; the scheduler parks
// the fiber and later drives `net-poll`. listen/close/local-addr/connect are
// synchronous (v1 connect blocks — a loopback dial completes in-kernel).
//
// Engine-independent: the V8 net callbacks in `v8host::net` shape these `NetRet`s
// into the marshalling ABI, but nothing here touches V8.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::{Arc, OnceLock};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore};
use socket2::{Domain, Socket, Type};

use crate::offload::{Interest, Reactor};

/// A TLS client connection: the underlying (non-blocking, after adoption) TCP socket
/// plus the rustls session driving it. The handshake is already complete by the time
/// one of these lands in the socket table — the offloaded `connect-tls` worker runs it
/// blocking (`tls_client_connect`), so the suspending read/write path here only ever
/// moves application records, never handshake messages.
///
/// `write_pending` guards the resume of a partial write: a `net.write` that couldn't
/// flush all its ciphertext parks and re-runs with the *same* plaintext, so the flag
/// says "the plaintext is already buffered in rustls — don't feed it again, just keep
/// flushing." Cleared once the ciphertext is fully drained.
pub(crate) struct TlsClient {
	sock: TcpStream,
	conn: Box<ClientConnection>,
	write_pending: bool,
}

/// A live socket the program holds a handle to (an opaque `int` id into `sockets`).
/// A `Tls` connection reads/writes exactly like a plain `Conn` from the caller's view —
/// the byte ops below transparently run the rustls record layer over it — so nothing
/// above `net.connect-tls` (the HTTP framing, the keep-alive loop) knows the difference.
enum SocketEntry {
	Listener(TcpListener),
	Conn(TcpStream),
	Tls(TlsClient),
}

impl SocketEntry {
	fn raw_fd(&self) -> RawFd {
		match self {
			SocketEntry::Listener(l) => l.as_raw_fd(),
			SocketEntry::Conn(c) => c.as_raw_fd(),
			SocketEntry::Tls(t) => t.sock.as_raw_fd(),
		}
	}
}

/// The outcome of one host net op, before it's shaped into a `result` `$value`.
/// `OkInt` rides the i32 `n` return channel (boxed in wasm); the value-bearing
/// arms build a primitive `$value` payload; `WouldBlock` signals a park.
pub(crate) enum NetRet {
	OkInt(i32), // a listener/connection id, or a bytes-written count
	OkBytes(Vec<u8>),
	OkStr(String),
	OkNothing,
	Err(String),
	WouldBlock,
}

/// The `std/sys/net` socket table. The readiness reactor (poller, parked fibers, the
/// `poll`/`unwatch` step) now lives in the shared `Reactor` (`crate::offload`) so socket
/// readiness and offload completion share one poll step; `HostNet` keeps only the socket
/// handles, and threads a `&mut Reactor` through the suspending ops to park on it.
#[derive(Default)]
pub(crate) struct HostNet {
	sockets: HashMap<u32, SocketEntry>,
	next_id: u32,
}

/// Bind a listening socket with `SO_REUSEADDR` set, the equivalent of std's
/// `TcpListener::bind` but tolerant of a just-closed predecessor. Without it, a
/// server that restarts while connections to its port are still draining in
/// `TIME_WAIT` (exactly what `pluma dev` does — it proxies long-lived SSE to the
/// server subprocess, then kills and respawns it on each edit) fails to rebind
/// with `EADDRINUSE`. `SO_REUSEADDR` is the standard server idiom for this; it
/// only permits reuse of a dead/closed address, not stealing a live listener.
fn bind_reusable(addr: &str) -> std::io::Result<TcpListener> {
	let sockaddr = addr
		.to_socket_addrs()?
		.next()
		.ok_or_else(|| std::io::Error::other(format!("could not resolve address `{addr}`")))?;
	let socket = Socket::new(Domain::for_address(sockaddr), Type::STREAM, None)?;
	socket.set_reuse_address(true)?;
	socket.bind(&sockaddr.into())?;
	socket.listen(1024)?;
	Ok(socket.into())
}

impl HostNet {
	fn store(&mut self, e: SocketEntry) -> u32 {
		let id = self.next_id;
		self.next_id += 1;
		self.sockets.insert(id, e);
		id
	}

	pub(crate) fn listen(&mut self, addr: &str) -> NetRet {
		match bind_reusable(addr) {
			Ok(l) => match l.set_nonblocking(true) {
				Ok(()) => NetRet::OkInt(self.store(SocketEntry::Listener(l)) as i32),
				Err(e) => NetRet::Err(e.to_string()),
			},
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	pub(crate) fn close(&mut self, id: u32) -> NetRet {
		match self.sockets.remove(&id) {
			Some(_) => NetRet::OkNothing,
			None => NetRet::Err(format!("net.close: no such socket ({id})")),
		}
	}

	pub(crate) fn local_addr(&self, id: u32) -> NetRet {
		let addr = match self.sockets.get(&id) {
			Some(SocketEntry::Listener(l)) => l.local_addr(),
			Some(SocketEntry::Conn(c)) => c.local_addr(),
			Some(SocketEntry::Tls(t)) => t.sock.local_addr(),
			None => return NetRet::Err(format!("net.local-addr: no such socket ({id})")),
		};
		match addr {
			Ok(a) => NetRet::OkStr(a.to_string()),
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	/// Adopt a connected stream (handed back by an offloaded `net.connect` worker — see
	/// `crate::offload`): set it non-blocking and store it, returning the socket id. Runs on
	/// the scheduler thread (the only one that touches the socket table) at collect time.
	pub(crate) fn adopt_conn(&mut self, stream: TcpStream) -> NetRet {
		match stream.set_nonblocking(true) {
			Ok(()) => NetRet::OkInt(self.store(SocketEntry::Conn(stream)) as i32),
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	/// Adopt a TLS client connection handed back by an offloaded `net.connect-tls` worker
	/// (which did the blocking dial + TLS handshake — see `tls_client_connect`): switch the
	/// now-handshaked socket to non-blocking and store it, returning the socket id. The
	/// resulting connection reads/writes through `read`/`write` like any other.
	pub(crate) fn adopt_tls_conn(&mut self, client: TlsClient) -> NetRet {
		match client.sock.set_nonblocking(true) {
			Ok(()) => NetRet::OkInt(self.store(SocketEntry::Tls(client)) as i32),
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	pub(crate) fn try_accept(&mut self, reactor: &mut Reactor, fid: i32, lid: u32) -> NetRet {
		let res = match self.sockets.get(&lid) {
			Some(SocketEntry::Listener(l)) => l.accept(),
			_ => return NetRet::Err(format!("net.accept: not a listener ({lid})")),
		};
		match res {
			Ok((stream, _peer)) => match stream.set_nonblocking(true) {
				Ok(()) => NetRet::OkInt(self.store(SocketEntry::Conn(stream)) as i32),
				Err(e) => NetRet::Err(e.to_string()),
			},
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
				self.park(reactor, fid, lid, Interest::Read)
			}
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	pub(crate) fn try_read(
		&mut self,
		reactor: &mut Reactor,
		fid: i32,
		cid: u32,
		max: usize,
	) -> NetRet {
		let mut buf = vec![0u8; max];
		let res = match self.sockets.get_mut(&cid) {
			Some(SocketEntry::Conn(c)) => c.read(&mut buf),
			Some(SocketEntry::Tls(t)) => return tls_read(reactor, fid, t, max),
			_ => return NetRet::Err(format!("net.read: not a connection ({cid})")),
		};
		match res {
			// n == 0 is a clean EOF: an empty `bytes`, distinguishable by length.
			Ok(n) => {
				buf.truncate(n);
				NetRet::OkBytes(buf)
			}
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
				self.park(reactor, fid, cid, Interest::Read)
			}
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	pub(crate) fn try_write(
		&mut self,
		reactor: &mut Reactor,
		fid: i32,
		cid: u32,
		data: &[u8],
	) -> NetRet {
		let res = match self.sockets.get_mut(&cid) {
			Some(SocketEntry::Conn(c)) => c.write(data),
			Some(SocketEntry::Tls(t)) => return tls_write(reactor, fid, t, data),
			_ => return NetRet::Err(format!("net.write: not a connection ({cid})")),
		};
		match res {
			Ok(n) => NetRet::OkInt(n as i32),
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
				self.park(reactor, fid, cid, Interest::Write)
			}
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	/// Register fiber `fid` against socket `sid`'s readiness on the shared reactor (token =
	/// fid), then report would-block.
	fn park(&self, reactor: &mut Reactor, fid: i32, sid: u32, interest: Interest) -> NetRet {
		let fd = match self.sockets.get(&sid) {
			Some(e) => e.raw_fd(),
			None => return NetRet::Err(format!("net: park on unknown socket {sid}")),
		};
		match reactor.register_socket(fid, fd, interest) {
			Ok(()) => NetRet::WouldBlock,
			Err(e) => NetRet::Err(e),
		}
	}
}

// --- TLS (`net.connect-tls`, https `http.fetch`) -------------------------------
//
// A TLS connection runs rustls' record layer over a non-blocking socket. The handshake
// is *not* here — `tls_client_connect` completes it blocking on the `connect-tls` worker
// before the connection is adopted — so these two ops only ever move application data,
// parking the fiber on socket readiness exactly like the plaintext path (`reactor` is
// passed straight through, so registering on it doesn't re-borrow the socket table).

/// Read up to `max` plaintext bytes from a TLS connection (the `Tls` arm of `net.read`).
/// Drains rustls' already-decrypted buffer first; only when that's empty does it pull
/// fresh ciphertext off the socket, decrypt it, and try again — parking on read-readiness
/// if the socket has nothing to give. A zero-length `OkBytes` is a clean end of stream
/// (peer `close_notify`, or the socket hitting EOF), the same empty-`bytes` signal the
/// plaintext `read` uses.
fn tls_read(reactor: &mut Reactor, fid: i32, t: &mut TlsClient, max: usize) -> NetRet {
	let mut buf = vec![0u8; max];
	loop {
		// 1. Hand back any plaintext rustls has already decrypted.
		match t.conn.reader().read(&mut buf) {
			Ok(n) => {
				buf.truncate(n);
				return NetRet::OkBytes(buf);
			}
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {} // need more ciphertext
			Err(e) => return NetRet::Err(e.to_string()),
		}
		// 2. Pull ciphertext off the socket and feed it to rustls.
		match t.conn.read_tls(&mut t.sock) {
			Ok(0) => return NetRet::OkBytes(Vec::new()), // socket EOF
			Ok(_) => {
				if let Err(e) = t.conn.process_new_packets() {
					return NetRet::Err(e.to_string());
				}
				// A post-handshake message (e.g. a TLS 1.3 key update) can leave rustls
				// wanting to write an acknowledgement; flush it best-effort so it isn't
				// stranded on a read-only request/response exchange.
				tls_flush_best_effort(t);
				// loop: try to decrypt+return, or pull more ciphertext
			}
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
				let fd = t.sock.as_raw_fd();
				return match reactor.register_socket(fid, fd, Interest::Read) {
					Ok(()) => NetRet::WouldBlock,
					Err(e) => NetRet::Err(e),
				};
			}
			Err(e) => return NetRet::Err(e.to_string()),
		}
	}
}

/// Write `data` to a TLS connection (the `Tls` arm of `net.write`), reporting all of it
/// as written once the encrypted bytes are fully flushed to the socket. rustls buffers the
/// plaintext, so on a would-block mid-flush the fiber parks and re-runs this with the same
/// `data`; `write_pending` keeps that resume from buffering the plaintext twice.
fn tls_write(reactor: &mut Reactor, fid: i32, t: &mut TlsClient, data: &[u8]) -> NetRet {
	if !t.write_pending {
		if let Err(e) = t.conn.writer().write_all(data) {
			return NetRet::Err(e.to_string());
		}
		t.write_pending = true;
	}
	while t.conn.wants_write() {
		match t.conn.write_tls(&mut t.sock) {
			Ok(_) => {}
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
				let fd = t.sock.as_raw_fd();
				return match reactor.register_socket(fid, fd, Interest::Write) {
					Ok(()) => NetRet::WouldBlock,
					Err(e) => NetRet::Err(e),
				};
			}
			Err(e) => return NetRet::Err(e.to_string()),
		}
	}
	t.write_pending = false;
	NetRet::OkInt(data.len() as i32)
}

/// Push out whatever ciphertext rustls has queued, stopping at the first would-block.
/// Used opportunistically after a read to drain control-message acknowledgements; a
/// would-block here is fine (the next `write` finishes the flush), so errors are ignored.
fn tls_flush_best_effort(t: &mut TlsClient) {
	while t.conn.wants_write() {
		match t.conn.write_tls(&mut t.sock) {
			Ok(_) => {}
			Err(_) => break,
		}
	}
}

/// The process-wide client TLS config: verify servers against the bundled Mozilla root
/// set (`webpki-roots`), no client certificate. Built once — assembling the root store and
/// the crypto config is non-trivial and the result is immutable and shareable. Pinned to
/// the `ring` provider so the config never depends on a process-global default provider
/// being installed.
fn default_client_config() -> Arc<ClientConfig> {
	static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
	CONFIG
		.get_or_init(|| {
			let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
			Arc::new(build_client_config(roots))
		})
		.clone()
}

/// Assemble a client config trusting `roots` (the production set, or a test's own CA).
fn build_client_config(roots: RootCertStore) -> ClientConfig {
	ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
		.with_safe_default_protocol_versions()
		.expect("ring provider supports the default TLS versions")
		.with_root_certificates(roots)
		.with_no_client_auth()
}

/// Dial `addr` (a `host:port`) and complete the TLS handshake, blocking — this runs on the
/// `connect-tls` offload worker, so blocking is fine and keeps the handshake off the
/// suspending read/write path. SNI is the host part of `addr`. Returns a handshaked
/// `TlsClient` the scheduler thread adopts (`adopt_tls_conn`). The `Result` mirrors plain
/// `connect`'s: `Err` carries the message, surfaced to Pluma as `err`.
pub(crate) fn tls_client_connect(addr: &str) -> Result<TlsClient, String> {
	tls_connect_with(addr, default_client_config())
}

/// `tls_client_connect`, but with an explicit config — the seam the integration test uses
/// to trust its self-signed loopback cert instead of the public roots.
fn tls_connect_with(addr: &str, config: Arc<ClientConfig>) -> Result<TlsClient, String> {
	// SNI / cert name = the host part of `host:port` (rsplit so an IPv6 literal's inner
	// colons stay with the host).
	let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
	let server_name = ServerName::try_from(host.to_string())
		.map_err(|e| format!("net.connect-tls: bad server name `{host}`: {e}"))?;
	let mut sock = TcpStream::connect(addr).map_err(|e| e.to_string())?;
	let mut conn =
		ClientConnection::new(config, server_name).map_err(|e| format!("net.connect-tls: {e}"))?;
	// Drive the handshake to completion on the (still blocking) socket.
	while conn.is_handshaking() {
		conn
			.complete_io(&mut sock)
			.map_err(|e| format!("net.connect-tls: handshake failed: {e}"))?;
	}
	Ok(TlsClient {
		sock,
		conn: Box::new(conn),
		write_pending: false,
	})
}

// --- std/web/fetch transport (the native/V8 host) ------------------------------
//
// In the browser the `web-fetch` host call is a synchronous `XMLHttpRequest`; for
// the V8 host it's a blocking HTTP/1.1 exchange over `std::net` (the engine-side
// counterpart, used by `tests/run` and `pluma run`). The wasm side marshals one
// request string in and reads one reply string out (`emit_web_fetch`); this is the
// engine-independent body the V8 callback (`v8host::net::cb_web_fetch`) wraps.

/// A blocking byte stream the request/response exchange runs over — either a plain
/// `TcpStream` or a rustls `StreamOwned` (which drives the TLS handshake lazily on the
/// first read/write), boxed so `web_fetch` is agnostic to which.
trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

/// Perform one blocking HTTP/1.1 request. `req` is `"<method>\t<url>\t<headers>\t
/// <hex-body>"` (headers as `k:v;k:v`); the reply is `"<status>\t<hex-body>"`. An
/// `https` URL goes over TLS (cert verified against the bundled roots), anything else
/// plain TCP; `Connection: close` either way. `Err` carries the message (stashed in
/// `last_error`, surfaced to Pluma as `err` via `__io_result`).
pub fn web_fetch(req: &str) -> Result<String, String> {
	let mut it = req.splitn(4, '\t');
	let method = it.next().unwrap_or("POST");
	let url = it.next().ok_or("web-fetch: malformed request")?;
	let headers = it.next().unwrap_or("");
	let body = hex_decode(it.next().unwrap_or("")).ok_or("web-fetch: bad hex body")?;
	let (secure, authority, path) = split_url(url);

	let sock = TcpStream::connect(&authority).map_err(|e| e.to_string())?;
	let mut stream: Box<dyn ReadWrite> = if secure {
		let host = authority
			.rsplit_once(':')
			.map(|(h, _)| h)
			.unwrap_or(&authority);
		let server_name = ServerName::try_from(host.to_string())
			.map_err(|e| format!("web-fetch: bad server name `{host}`: {e}"))?;
		let conn = ClientConnection::new(default_client_config(), server_name)
			.map_err(|e| format!("web-fetch: {e}"))?;
		Box::new(rustls::StreamOwned::new(conn, sock))
	} else {
		Box::new(sock)
	};

	let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {authority}\r\n");
	for h in headers.split(';').filter(|h| !h.is_empty()) {
		if let Some(i) = h.find(':') {
			head.push_str(&format!("{}: {}\r\n", &h[..i], &h[i + 1..]));
		}
	}
	head.push_str(&format!("Content-Length: {}\r\n", body.len()));
	head.push_str("Connection: close\r\n\r\n");
	let mut wire = head.into_bytes();
	wire.extend_from_slice(&body);
	stream.write_all(&wire).map_err(|e| e.to_string())?;
	stream.flush().map_err(|e| e.to_string())?;

	// `Connection: close` → read to EOF, then split off the header block.
	let mut resp = Vec::new();
	stream.read_to_end(&mut resp).map_err(|e| e.to_string())?;
	let (status, resp_body) = parse_http_response(&resp).ok_or("web-fetch: malformed response")?;
	Ok(format!("{status}\t{}", hex_encode(&resp_body)))
}

/// Split a URL into `(secure, "host:port", "/path?query")`. The scheme prefix
/// (`http://` or `https://`) is stripped, `secure` reflects `https`, a missing path
/// defaults to `/`, and a missing port defaults to 80 (or 443 under `https`) since
/// `TcpStream::connect` needs an explicit `host:port`.
fn split_url(url: &str) -> (bool, String, String) {
	let (secure, rest) = match url.strip_prefix("https://") {
		Some(r) => (true, r),
		None => (false, url.strip_prefix("http://").unwrap_or(url)),
	};
	let (authority, path) = match rest.find('/') {
		Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
		None => (rest.to_string(), "/".to_string()),
	};
	let authority = if authority.contains(':') {
		authority
	} else if secure {
		format!("{authority}:443")
	} else {
		format!("{authority}:80")
	};
	(secure, authority, path)
}

/// Parse `(status, body)` out of a raw HTTP/1.1 response: the status code from the
/// first line, the body as everything after the blank `\r\n\r\n` separator.
fn parse_http_response(resp: &[u8]) -> Option<(u16, Vec<u8>)> {
	let sep = resp.windows(4).position(|w| w == b"\r\n\r\n")?;
	let head = &resp[..sep];
	let body = resp[sep + 4..].to_vec();
	let first = head.split(|&b| b == b'\r').next().unwrap_or(head);
	let status = std::str::from_utf8(first)
		.ok()?
		.split(' ')
		.nth(1)?
		.parse()
		.ok()?;
	Some((status, body))
}

fn hex_encode(bytes: &[u8]) -> String {
	let mut s = String::with_capacity(bytes.len() * 2);
	for b in bytes {
		s.push_str(&format!("{b:02x}"));
	}
	s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
	if s.len() % 2 != 0 {
		return None;
	}
	(0..s.len())
		.step_by(2)
		.map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
		.collect()
}

#[cfg(test)]
mod tests {
	use std::io::{Read, Write};
	use std::net::TcpListener;
	use std::sync::Arc;
	use std::thread;

	use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
	use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};

	use super::{HostNet, NetRet, build_client_config, tls_connect_with};
	use crate::offload::Reactor;

	/// End-to-end TLS over loopback, hermetic (no network, both ends the test's own):
	/// a rustls echo server with a freshly generated self-signed cert, and the real
	/// client path — `tls_connect_with` (blocking handshake) then `HostNet`'s
	/// non-blocking `try_write`/`try_read` driven through a `Reactor` exactly as the
	/// scheduler drives them, parking on would-block and waking on `poll`.
	#[test]
	fn tls_client_roundtrip() {
		// A self-signed cert valid for 127.0.0.1 (the SAN is parsed as an IP).
		let ck = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
		let cert_der: CertificateDer<'static> = ck.cert.der().clone();
		let key_der = PrivatePkcs8KeyDer::from(ck.key_pair.serialize_der());

		// rustls echo server: accept one connection, read a line, write it back uppercased.
		let server_config = ServerConfig::builder_with_provider(Arc::new(
			rustls::crypto::ring::default_provider(),
		))
		.with_safe_default_protocol_versions()
		.unwrap()
		.with_no_client_auth()
		.with_single_cert(vec![cert_der.clone()], key_der.into())
		.unwrap();
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let addr = listener.local_addr().unwrap().to_string();
		let server_config = Arc::new(server_config);
		let server = thread::spawn(move || {
			let (sock, _) = listener.accept().unwrap();
			let conn = ServerConnection::new(server_config).unwrap();
			let mut tls = StreamOwned::new(conn, sock);
			let mut buf = [0u8; 64];
			let n = tls.read(&mut buf).unwrap(); // drives the handshake, then reads app data
			let upper: Vec<u8> = buf[..n].iter().map(u8::to_ascii_uppercase).collect();
			tls.write_all(&upper).unwrap();
			tls.flush().unwrap();
		});

		// Client: trust the test cert, dial + handshake (blocking, as the offload worker does).
		let mut roots = RootCertStore::empty();
		roots.add(cert_der).unwrap();
		let config = Arc::new(build_client_config(roots));
		let client = tls_connect_with(&addr, config).expect("tls handshake");

		// Adopt it into a socket table and exercise the suspending byte ops through a reactor.
		let mut net = HostNet::default();
		let cid = match net.adopt_tls_conn(client) {
			NetRet::OkInt(n) => n as u32,
			other => panic!("adopt: {}", describe(&other)),
		};
		let mut reactor = Reactor::default();
		let fid = 1;

		send_all(&mut net, &mut reactor, fid, cid, b"hello\n");
		let got = recv_line(&mut net, &mut reactor, fid, cid);
		assert_eq!(got, b"HELLO\n");
		server.join().unwrap();
	}

	/// Write every byte, re-driving `try_write` after each park (would-block → poll → retry),
	/// the same loop the scheduler runs around a `wait::IO` fiber.
	fn send_all(net: &mut HostNet, reactor: &mut Reactor, fid: i32, cid: u32, msg: &[u8]) {
		let mut sent = 0;
		while sent < msg.len() {
			match net.try_write(reactor, fid, cid, &msg[sent..]) {
				NetRet::OkInt(n) => sent += n as usize,
				NetRet::WouldBlock => assert_eq!(reactor.poll(-1), fid, "woke wrong fiber"),
				other => panic!("write: {}", describe(&other)),
			}
		}
	}

	/// Read until a newline (or EOF), parking on would-block just like the scheduler.
	fn recv_line(net: &mut HostNet, reactor: &mut Reactor, fid: i32, cid: u32) -> Vec<u8> {
		let mut got = Vec::new();
		loop {
			match net.try_read(reactor, fid, cid, 1024) {
				NetRet::OkBytes(b) if b.is_empty() => return got, // EOF
				NetRet::OkBytes(b) => {
					got.extend_from_slice(&b);
					if got.ends_with(b"\n") {
						return got;
					}
				}
				NetRet::WouldBlock => assert_eq!(reactor.poll(-1), fid, "woke wrong fiber"),
				other => panic!("read: {}", describe(&other)),
			}
		}
	}

	fn describe(r: &NetRet) -> String {
		match r {
			NetRet::Err(e) => format!("err: {e}"),
			_ => "unexpected NetRet".to_string(),
		}
	}
}
