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

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
	ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, ServerConfig,
	ServerConnection, SignatureScheme,
};
use socket2::{Domain, Socket, Type};

use crate::offload::{Interest, Reactor};

/// A TLS connection — client or server — as the underlying (non-blocking) TCP socket plus
/// the rustls session driving it (`rustls::Connection` unifies `ClientConnection` and
/// `ServerConnection`, so one record-layer driver serves both ends). The handshake is run
/// *lazily* by `tls_read`/`tls_write` on first use, not up front: those drivers move
/// whatever TLS records rustls wants and park on the matching readiness, so a handshake in
/// flight is just records that happen to precede application data — no separate phase, and
/// no worker thread pinned for a handshake round-trip.
///
/// `write_pending` guards the resume of a partial write: a `net.write` that couldn't flush
/// all its ciphertext parks and re-runs with the *same* plaintext, so the flag says "the
/// plaintext is already buffered in rustls — don't feed it again, just keep flushing."
/// Cleared once the ciphertext is fully drained.
pub(crate) struct TlsConn {
	sock: TcpStream,
	conn: rustls::Connection,
	write_pending: bool,
}

/// A live socket the program holds a handle to (an opaque `int` id into `sockets`).
/// A `Tls` connection reads/writes exactly like a plain `Conn` from the caller's view —
/// the byte ops below transparently run the rustls record layer over it — so nothing
/// above `net.connect-tls` / a `net.listen-tls` listener (the HTTP framing, the keep-alive
/// loop) knows the difference. A `TlsListener` carries the server config every connection
/// it accepts is wrapped in.
enum SocketEntry {
	Listener(TcpListener),
	TlsListener(TcpListener, Arc<ServerConfig>),
	Conn(TcpStream),
	Tls(TlsConn),
}

impl SocketEntry {
	fn raw_fd(&self) -> RawFd {
		match self {
			SocketEntry::Listener(l) => l.as_raw_fd(),
			SocketEntry::TlsListener(l, _) => l.as_raw_fd(),
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

	/// Bind a TLS-terminating listener. `spec` is `"addr\tcert-pem\tkey-pem"` (PEM never
	/// contains a tab); the cert+key build the rustls `ServerConfig` every accepted
	/// connection is wrapped in. Connections accepted off it are `Tls` and handshake lazily
	/// on first read/write, so `accept`/`read`/`write` are unchanged from the plaintext path.
	pub(crate) fn listen_tls(&mut self, spec: &str) -> NetRet {
		let mut parts = spec.splitn(3, '\t');
		let addr = parts.next().unwrap_or("");
		let cert_pem = parts.next().unwrap_or("");
		let key_pem = parts.next().unwrap_or("");
		let config = match build_server_config(cert_pem, key_pem) {
			Ok(c) => c,
			Err(e) => return NetRet::Err(e),
		};
		let listener = match bind_reusable(addr).and_then(|l| l.set_nonblocking(true).map(|()| l)) {
			Ok(l) => l,
			Err(e) => return NetRet::Err(e.to_string()),
		};
		NetRet::OkInt(self.store(SocketEntry::TlsListener(listener, config)) as i32)
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
			Some(SocketEntry::TlsListener(l, _)) => l.local_addr(),
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

	/// Wrap a freshly connected stream (handed back by the offloaded `net.connect-tls`
	/// worker, which did only the blocking dial) in a client-side TLS session and store it,
	/// non-blocking, returning the socket id. `server_name` is the SNI / cert name; `config`
	/// carries the trust roots. The handshake runs lazily on first read/write, like the
	/// server side — no handshake on the worker, so no pool thread is pinned for it.
	pub(crate) fn adopt_tls_client(
		&mut self,
		stream: TcpStream,
		server_name: ServerName<'static>,
		config: Arc<ClientConfig>,
	) -> NetRet {
		let conn = match ClientConnection::new(config, server_name) {
			Ok(c) => rustls::Connection::Client(c),
			Err(e) => return NetRet::Err(format!("net.connect-tls: {e}")),
		};
		self.adopt_tls(stream, conn)
	}

	/// Store `conn` (a client- or server-side rustls session) over `stream`, set non-blocking,
	/// return the socket id.
	fn adopt_tls(&mut self, stream: TcpStream, conn: rustls::Connection) -> NetRet {
		match stream.set_nonblocking(true) {
			Ok(()) => NetRet::OkInt(self.store(SocketEntry::Tls(TlsConn {
				sock: stream,
				conn,
				write_pending: false,
			})) as i32),
			Err(e) => NetRet::Err(e.to_string()),
		}
	}

	pub(crate) fn try_accept(&mut self, reactor: &mut Reactor, fid: i32, lid: u32) -> NetRet {
		// A TLS listener wraps each accepted stream in a server session (config cloned out so
		// the borrow of the table ends before we store the new connection). The handshake is
		// deferred to the first read/write — accept itself never blocks on it.
		let (res, tls_config) = match self.sockets.get(&lid) {
			Some(SocketEntry::Listener(l)) => (l.accept(), None),
			Some(SocketEntry::TlsListener(l, cfg)) => (l.accept(), Some(cfg.clone())),
			_ => return NetRet::Err(format!("net.accept: not a listener ({lid})")),
		};
		match res {
			Ok((stream, _peer)) => match tls_config {
				Some(cfg) => match ServerConnection::new(cfg) {
					Ok(c) => self.adopt_tls(stream, rustls::Connection::Server(c)),
					Err(e) => NetRet::Err(format!("net.accept: {e}")),
				},
				None => match stream.set_nonblocking(true) {
					Ok(()) => NetRet::OkInt(self.store(SocketEntry::Conn(stream)) as i32),
					Err(e) => NetRet::Err(e.to_string()),
				},
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

// --- TLS (`net.connect-tls`, `net.listen-tls`, https `http.fetch`) -------------
//
// A TLS connection runs rustls' record layer over a non-blocking socket. `tls_read`/
// `tls_write` are one non-blocking driver shared by both ends (the session is a
// `rustls::Connection`, client or server): each moves whatever TLS records rustls wants
// and parks the fiber on the *matching* readiness. The handshake isn't a separate phase —
// it's just the records that flow before application data — so it completes lazily across
// these calls with no worker thread pinned and no risk of the offload pool deadlocking on
// mutually-blocked handshakes. `reactor` is a separate borrow from the socket table, so
// parking on it never re-borrows `self.sockets`.

/// Park `fid` on `t`'s socket becoming ready for `interest`. Used by the TLS driver, which
/// holds a `&mut TlsConn` borrowed out of the table and so can't call `HostNet::park`.
fn tls_park(reactor: &mut Reactor, fid: i32, t: &TlsConn, interest: Interest) -> NetRet {
	match reactor.register_socket(fid, t.sock.as_raw_fd(), interest) {
		Ok(()) => NetRet::WouldBlock,
		Err(e) => NetRet::Err(e),
	}
}

/// Flush every TLS record rustls currently wants to send, parking on write-readiness if the
/// socket fills. Drives both handshake output (ServerHello, Finished, …) and queued
/// application ciphertext. `Ok(false)` = more to flush but the socket blocked (parked);
/// `Ok(true)` = nothing left to write.
fn tls_pump_writes(reactor: &mut Reactor, fid: i32, t: &mut TlsConn) -> Result<bool, NetRet> {
	while t.conn.wants_write() {
		match t.conn.write_tls(&mut t.sock) {
			Ok(_) => {}
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
				return Err(tls_park(reactor, fid, t, Interest::Write));
			}
			Err(e) => return Err(NetRet::Err(e.to_string())),
		}
	}
	Ok(true)
}

/// Pull one batch of ciphertext off the socket and feed it to rustls. `Ok(true)` = bytes
/// were ingested (caller should retry decrypting); `Ok(false)` = clean socket EOF. A
/// would-block parks on read-readiness (returned via `Err`).
fn tls_pump_read(reactor: &mut Reactor, fid: i32, t: &mut TlsConn) -> Result<bool, NetRet> {
	match t.conn.read_tls(&mut t.sock) {
		Ok(0) => Ok(false), // socket EOF
		Ok(_) => match t.conn.process_new_packets() {
			Ok(_) => Ok(true),
			Err(e) => Err(NetRet::Err(e.to_string())),
		},
		Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
			Err(tls_park(reactor, fid, t, Interest::Read))
		}
		Err(e) => Err(NetRet::Err(e.to_string())),
	}
}

/// Read up to `max` plaintext bytes from a TLS connection (the `Tls` arm of `net.read`).
/// Flushes any pending TLS output first (so a handshake reply or key-update ack isn't
/// stranded), hands back already-decrypted plaintext, and otherwise pulls + decrypts more
/// ciphertext — parking on the right readiness as it goes. A zero-length `OkBytes` is a
/// clean end of stream (peer `close_notify`, or socket EOF), the same empty-`bytes` signal
/// the plaintext `read` uses.
fn tls_read(reactor: &mut Reactor, fid: i32, t: &mut TlsConn, max: usize) -> NetRet {
	let mut buf = vec![0u8; max];
	loop {
		if let Err(park) = tls_pump_writes(reactor, fid, t) {
			return park;
		}
		match t.conn.reader().read(&mut buf) {
			Ok(n) => {
				buf.truncate(n);
				return NetRet::OkBytes(buf);
			}
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {} // need more ciphertext
			Err(e) => return NetRet::Err(e.to_string()),
		}
		match tls_pump_read(reactor, fid, t) {
			Ok(true) => {}                                   // ingested — retry decrypt
			Ok(false) => return NetRet::OkBytes(Vec::new()), // EOF
			Err(park) => return park,
		}
	}
}

/// Write `data` to a TLS connection (the `Tls` arm of `net.write`), reporting all of it as
/// written once the encrypted bytes are fully flushed. rustls buffers the plaintext, so on
/// a would-block mid-flush the fiber parks and re-runs this with the same `data`;
/// `write_pending` keeps that resume from buffering the plaintext twice. If the session is
/// still handshaking, the driver reads peer records as needed so a write issued before the
/// handshake settles still makes progress.
fn tls_write(reactor: &mut Reactor, fid: i32, t: &mut TlsConn, data: &[u8]) -> NetRet {
	if !t.write_pending {
		if let Err(e) = t.conn.writer().write_all(data) {
			return NetRet::Err(e.to_string());
		}
		t.write_pending = true;
	}
	loop {
		if let Err(park) = tls_pump_writes(reactor, fid, t) {
			return park;
		}
		// Flushed everything rustls had queued. If a handshake is still in flight it's now
		// our turn to read the peer's next flight; otherwise the write is complete.
		if t.conn.is_handshaking() && t.conn.wants_read() {
			match tls_pump_read(reactor, fid, t) {
				Ok(true) => continue, // advanced the handshake — loop to flush our reply
				Ok(false) => return NetRet::Err("net.write: connection closed mid-handshake".into()),
				Err(park) => return park,
			}
		}
		t.write_pending = false;
		return NetRet::OkInt(data.len() as i32);
	}
}

/// The process-wide client TLS config: verify servers against the bundled Mozilla root set
/// (`webpki-roots`), no client certificate. Built once — assembling the root store and the
/// crypto config is non-trivial and the result is immutable and shareable. Pinned to the
/// `ring` provider so the config never depends on a process-global default provider being
/// installed.
fn default_client_config() -> Arc<ClientConfig> {
	static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
	CONFIG
		.get_or_init(|| Arc::new(build_client_config(public_roots())))
		.clone()
}

/// The bundled Mozilla trust anchors as a fresh root store.
fn public_roots() -> RootCertStore {
	RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned())
}

/// Assemble a client config trusting `roots`, pinned to the `ring` provider.
fn build_client_config(roots: RootCertStore) -> ClientConfig {
	ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
		.with_safe_default_protocol_versions()
		.expect("ring provider supports the default TLS versions")
		.with_root_certificates(roots)
		.with_no_client_auth()
}

/// The client config for a connect: the cached public-roots config when no extra trust is
/// pinned, else a fresh config that trusts the certificate(s) in `ca_pem` in addition to
/// the public roots. This is what `connect-tls-trusting` reaches for self-signed / private
/// servers, and what makes a hermetic loopback test possible. Building a verifier
/// per-connect is fine — connects are rare next to I/O.
fn client_config_for(ca_pem: Option<&str>) -> Result<Arc<ClientConfig>, String> {
	let Some(pem) = ca_pem else {
		return Ok(default_client_config());
	};
	let pinned = parse_certs(pem)?;
	if pinned.is_empty() {
		return Err("net.connect-tls: trust anchor PEM held no certificate".into());
	}
	// Two trust paths, unioned: webpki chain validation against the public roots *plus* the
	// provided certs as anchors (so a real internal CA, which signs a separate leaf,
	// validates normally), and exact-leaf pinning of the provided certs (so a *self-signed*
	// server cert is trusted even though webpki rejects a CA-flagged cert used as a leaf).
	let provider = Arc::new(rustls::crypto::ring::default_provider());
	let mut roots = public_roots();
	roots.add_parsable_certificates(pinned.clone());
	let webpki = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
		.build()
		.map_err(|e| format!("net.connect-tls: {e}"))?;
	let verifier = Arc::new(PinnedOrWebPki { pinned, webpki });
	let config = ClientConfig::builder_with_provider(provider)
		.with_safe_default_protocol_versions()
		.expect("ring provider supports the default TLS versions")
		.dangerous()
		.with_custom_certificate_verifier(verifier)
		.with_no_client_auth();
	Ok(Arc::new(config))
}

/// A server-cert verifier that trusts a connection if the server's leaf certificate exactly
/// matches one we were told to pin, and otherwise defers to standard webpki chain validation
/// (`net.connect-tls-trusting`). Pinning is what lets a self-signed cert through — webpki
/// alone rejects a CA-flagged certificate presented as the leaf. The handshake-signature
/// checks always go through webpki, so even a pinned server still has to prove it holds the
/// matching private key.
#[derive(Debug)]
struct PinnedOrWebPki {
	pinned: Vec<CertificateDer<'static>>,
	webpki: Arc<WebPkiServerVerifier>,
}

impl ServerCertVerifier for PinnedOrWebPki {
	fn verify_server_cert(
		&self,
		end_entity: &CertificateDer<'_>,
		intermediates: &[CertificateDer<'_>],
		server_name: &ServerName<'_>,
		ocsp_response: &[u8],
		now: UnixTime,
	) -> Result<ServerCertVerified, rustls::Error> {
		if self
			.pinned
			.iter()
			.any(|c| c.as_ref() == end_entity.as_ref())
		{
			return Ok(ServerCertVerified::assertion());
		}
		self
			.webpki
			.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
	}

	fn verify_tls12_signature(
		&self,
		message: &[u8],
		cert: &CertificateDer<'_>,
		dss: &DigitallySignedStruct,
	) -> Result<HandshakeSignatureValid, rustls::Error> {
		self.webpki.verify_tls12_signature(message, cert, dss)
	}

	fn verify_tls13_signature(
		&self,
		message: &[u8],
		cert: &CertificateDer<'_>,
		dss: &DigitallySignedStruct,
	) -> Result<HandshakeSignatureValid, rustls::Error> {
		self.webpki.verify_tls13_signature(message, cert, dss)
	}

	fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
		self.webpki.supported_verify_schemes()
	}
}

/// Build a server config from a PEM cert chain + private key (the `net.listen-tls` inputs).
fn build_server_config(cert_pem: &str, key_pem: &str) -> Result<Arc<ServerConfig>, String> {
	let certs = parse_certs(cert_pem)?;
	if certs.is_empty() {
		return Err("net.listen-tls: certificate PEM held no certificate".into());
	}
	let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())
		.map_err(|e| format!("net.listen-tls: bad key PEM: {e}"))?
		.ok_or("net.listen-tls: key PEM held no private key")?;
	ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
		.with_safe_default_protocol_versions()
		.expect("ring provider supports the default TLS versions")
		.with_no_client_auth()
		.with_single_cert(certs, key)
		.map(Arc::new)
		.map_err(|e| format!("net.listen-tls: {e}"))
}

/// Parse a PEM blob into DER certificates.
fn parse_certs(pem: &str) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, String> {
	rustls_pemfile::certs(&mut pem.as_bytes())
		.collect::<Result<Vec<_>, _>>()
		.map_err(|e| format!("bad certificate PEM: {e}"))
}

/// Split a `net.connect-tls` spec — `"host:port"` or `"host:port\tca-pem"` — into the dial
/// address and the optional pinned trust anchor.
pub(crate) fn split_tls_spec(spec: &str) -> (&str, Option<&str>) {
	match spec.split_once('\t') {
		Some((addr, ca)) => (addr, Some(ca)),
		None => (spec, None),
	}
}

/// The SNI / certificate name to verify a server against: the host part of `host:port`
/// (rsplit so an IPv6 literal's inner colons stay with the host).
pub(crate) fn tls_server_name(addr: &str) -> Result<ServerName<'static>, String> {
	let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
	ServerName::try_from(host.to_string())
		.map_err(|e| format!("net.connect-tls: bad server name `{host}`: {e}"))
}

/// Build the client config a connect should use from its spec's optional CA (the seam the
/// blocking `web-fetch` path and the async connect callback share).
pub(crate) fn tls_client_config(ca_pem: Option<&str>) -> Result<Arc<ClientConfig>, String> {
	client_config_for(ca_pem)
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
