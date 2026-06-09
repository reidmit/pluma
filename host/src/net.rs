// `std.sys.net` — the host-side socket table + I/O reactor: byte-level TCP ops plus a
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

use socket2::{Domain, Socket, Type};

use crate::offload::{Interest, Reactor};

/// A live socket the program holds a handle to (an opaque `int` id into `sockets`).
enum SocketEntry {
	Listener(TcpListener),
	Conn(TcpStream),
}

impl SocketEntry {
	fn raw_fd(&self) -> RawFd {
		match self {
			SocketEntry::Listener(l) => l.as_raw_fd(),
			SocketEntry::Conn(c) => c.as_raw_fd(),
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

/// The `std.sys.net` socket table. The readiness reactor (poller, parked fibers, the
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
			None => return NetRet::Err(format!("net.local-addr: no such socket ({id})")),
		};
		match addr {
			Ok(a) => NetRet::OkStr(a.to_string()),
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	pub(crate) fn connect(&mut self, addr: &str) -> NetRet {
		match TcpStream::connect(addr) {
			Ok(s) => match s.set_nonblocking(true) {
				Ok(()) => NetRet::OkInt(self.store(SocketEntry::Conn(s)) as i32),
				Err(e) => NetRet::Err(e.to_string()),
			},
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

// --- std.web.fetch transport (the native/V8 host) ------------------------------
//
// In the browser the `web-fetch` host call is a synchronous `XMLHttpRequest`; for
// the V8 host it's a blocking HTTP/1.1 exchange over `std::net` (the engine-side
// counterpart, used by `tests/run` and `pluma run`). The wasm side marshals one
// request string in and reads one reply string out (`emit_web_fetch`); this is the
// engine-independent body the V8 callback (`v8host::net::cb_web_fetch`) wraps.

/// Perform one blocking HTTP/1.1 request. `req` is `"<method>\t<url>\t<headers>\t
/// <hex-body>"` (headers as `k:v;k:v`); the reply is `"<status>\t<hex-body>"`. Plain
/// TCP only (no TLS), `Connection: close`. `Err` carries the message (stashed in
/// `last_error`, surfaced to Pluma as `err` via `__io_result`).
pub fn web_fetch(req: &str) -> Result<String, String> {
	let mut it = req.splitn(4, '\t');
	let method = it.next().unwrap_or("POST");
	let url = it.next().ok_or("web-fetch: malformed request")?;
	let headers = it.next().unwrap_or("");
	let body = hex_decode(it.next().unwrap_or("")).ok_or("web-fetch: bad hex body")?;
	let (authority, path) = split_url(url);

	let mut stream = TcpStream::connect(&authority).map_err(|e| e.to_string())?;
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

/// Split `"http://host:port/a/b?x"` into `("host:port", "/a/b?x")` (the `http://`
/// scheme prefix is stripped; a missing path defaults to `/`).
fn split_url(url: &str) -> (String, String) {
	let rest = url.strip_prefix("http://").unwrap_or(url);
	match rest.find('/') {
		Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
		None => (rest.to_string(), "/".to_string()),
	}
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
