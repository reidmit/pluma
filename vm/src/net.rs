// core.net: the byte-level TCP socket primitive plus the scheduler's I/O reactor.
//
// This is the only OS-touching, raw part of networking — HTTP and everything
// above it is pure Pluma over these six builtins (see notes/NET.md). The ops
// that wait on the network (`accept`/`read`/`write`) are `task`s: each attempts
// one non-blocking syscall and, if it would block, *parks the calling fiber*
// against the socket's readiness rather than blocking the scheduler thread. The
// reactor here is what the scheduler's block step polls when the ready queue
// empties and socket I/O is in flight (see `vm::task`'s `block_until_ready`).
//
// Handles (`listener`/`connection`) are opaque Pluma enum types; at runtime each
// is just an integer id into `sockets`. `listen`/`close`/`connect` run
// synchronously (bind, teardown, and — in v1 — connect don't park).

use crate::value::{Value, VariantData};
use crate::vm::{RuntimeError, VM};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::rc::Rc;
use std::time::Duration;

use polling::{Event, Events, Poller};

// A live socket the program holds a handle to.
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

// A fiber parked on socket readiness, keyed by a reactor token.
struct IoWait {
	fid: usize,
	fd: RawFd,
	// The task to re-run when the socket is ready — it re-attempts the op (which
	// now succeeds, or parks again on a spurious wakeup). Re-running the original
	// op task is correct because a `WouldBlock` consumed nothing.
	retry: Value,
}

// Read- vs write-readiness for a park.
#[derive(Clone, Copy)]
pub(crate) enum Interest {
	Read,
	Write,
}

// The outcome of one non-blocking socket attempt: either it produced a value
// (`result …`), or it would block and the fiber must park for `Interest` on the
// given socket id.
pub(crate) enum IoStep {
	Ready(Value),
	Block(u32, Interest),
}

// All networking state: the socket table plus the readiness reactor. Lives on
// the VM (not the per-run `Scheduler`) so the `Poller` and any open sockets
// persist across the whole run.
pub(crate) struct NetState {
	sockets: HashMap<u32, SocketEntry>,
	next_id: u32,
	// Created lazily on the first park — a sync / sleep-only program never makes
	// one.
	poller: Option<Poller>,
	events: Events,
	waits: HashMap<usize, IoWait>,
	next_token: usize,
}

impl Default for NetState {
	fn default() -> Self {
		NetState {
			sockets: HashMap::new(),
			next_id: 0,
			poller: None,
			events: Events::new(),
			waits: HashMap::new(),
			next_token: 0,
		}
	}
}

// `result` whose `err` side is an OS message string — matching `core.io`'s
// convention that every fallible op returns `result _ string`.
fn ok(v: Value) -> Value {
	result(true, v)
}
fn err(msg: String) -> Value {
	result(false, Value::String(Rc::new(msg)))
}
fn result(is_ok: bool, payload: Value) -> Value {
	Value::Variant(Rc::new(VariantData {
		qualified_enum: Rc::new("__prelude__.result".to_string()),
		variant: Rc::new(if is_ok { "ok" } else { "err" }.to_string()),
		payload: vec![payload],
	}))
}

impl VM {
	// --- synchronous builtins -----------------------------------------------

	// `net.listen addr` — bind + listen, non-blocking. The backlog is set up
	// immediately; only `accept` waits.
	pub(crate) fn net_listen(&mut self, addr: &str) -> Value {
		match TcpListener::bind(addr) {
			Ok(l) => {
				if let Err(e) = l.set_nonblocking(true) {
					return err(e.to_string());
				}
				ok(Value::Int(self.net_store(SocketEntry::Listener(l)) as i64))
			}
			Err(e) => err(e.to_string()),
		}
	}

	// `net.close c` — drop the socket, closing the OS fd.
	pub(crate) fn net_close(&mut self, id: u32) -> Value {
		match self.net.sockets.remove(&id) {
			Some(_) => ok(Value::Nothing),
			None => err(format!("net.close: no such socket ({id})")),
		}
	}

	// `net.connect addr` — v1 uses a *blocking* connect (see notes/NET.md). The
	// load-bearing non-blocking paths are accept/read/write; outbound connect is
	// off the server hot path and, to a listening loopback socket, completes at
	// the kernel level without the acceptor running — so it can't deadlock the
	// single-threaded scheduler. A non-blocking connect is a noted follow-up.
	pub(crate) fn net_connect(&mut self, addr: &str) -> Value {
		match TcpStream::connect(addr) {
			Ok(s) => {
				if let Err(e) = s.set_nonblocking(true) {
					return err(e.to_string());
				}
				ok(Value::Int(self.net_store(SocketEntry::Conn(s)) as i64))
			}
			Err(e) => err(e.to_string()),
		}
	}

	// `net.local-addr l` — the address the socket is actually bound to. Lets a
	// caller bind to port 0 (system-assigned) and discover the chosen port.
	pub(crate) fn net_local_addr(&self, id: u32) -> Value {
		let addr = match self.net.sockets.get(&id) {
			Some(SocketEntry::Listener(l)) => l.local_addr(),
			Some(SocketEntry::Conn(c)) => c.local_addr(),
			None => return err(format!("net.local-addr: no such socket ({id})")),
		};
		match addr {
			Ok(a) => ok(Value::String(Rc::new(a.to_string()))),
			Err(e) => err(e.to_string()),
		}
	}

	fn net_store(&mut self, e: SocketEntry) -> u32 {
		let id = self.net.next_id;
		self.net.next_id += 1;
		self.net.sockets.insert(id, e);
		id
	}

	// --- suspending builtins (one non-blocking attempt) ---------------------

	pub(crate) fn net_try_accept(&mut self, lid: u32) -> IoStep {
		let listener = match self.net.sockets.get(&lid) {
			Some(SocketEntry::Listener(l)) => l,
			_ => return IoStep::Ready(err(format!("net.accept: not a listener ({lid})"))),
		};
		match listener.accept() {
			Ok((stream, _peer)) => {
				if let Err(e) = stream.set_nonblocking(true) {
					return IoStep::Ready(err(e.to_string()));
				}
				let id = self.net_store(SocketEntry::Conn(stream));
				IoStep::Ready(ok(Value::Int(id as i64)))
			}
			Err(e) if e.kind() == ErrorKind::WouldBlock => IoStep::Block(lid, Interest::Read),
			Err(e) => IoStep::Ready(err(e.to_string())),
		}
	}

	pub(crate) fn net_try_read(&mut self, cid: u32, max: usize) -> IoStep {
		let stream = match self.net.sockets.get_mut(&cid) {
			Some(SocketEntry::Conn(c)) => c,
			_ => return IoStep::Ready(err(format!("net.read: not a connection ({cid})"))),
		};
		let mut buf = vec![0u8; max];
		match stream.read(&mut buf) {
			// n == 0 is a clean EOF: an empty `bytes`, distinguishable by length.
			Ok(n) => {
				buf.truncate(n);
				IoStep::Ready(ok(Value::Bytes(Rc::new(buf))))
			}
			Err(e) if e.kind() == ErrorKind::WouldBlock => IoStep::Block(cid, Interest::Read),
			Err(e) => IoStep::Ready(err(e.to_string())),
		}
	}

	pub(crate) fn net_try_write(&mut self, cid: u32, data: &[u8]) -> IoStep {
		let stream = match self.net.sockets.get_mut(&cid) {
			Some(SocketEntry::Conn(c)) => c,
			_ => return IoStep::Ready(err(format!("net.write: not a connection ({cid})"))),
		};
		match stream.write(data) {
			Ok(n) => IoStep::Ready(ok(Value::Int(n as i64))),
			Err(e) if e.kind() == ErrorKind::WouldBlock => IoStep::Block(cid, Interest::Write),
			Err(e) => IoStep::Ready(err(e.to_string())),
		}
	}

	// --- the reactor --------------------------------------------------------

	pub(crate) fn net_has_io_waits(&self) -> bool {
		!self.net.waits.is_empty()
	}

	// Register fiber `fid` to be re-run (`retry`) when socket `sid` is ready for
	// `interest`. Returns the reactor token, stored in the fiber's `Wait::Io`.
	pub(crate) fn reactor_park(
		&mut self,
		fid: usize,
		sid: u32,
		interest: Interest,
		retry: Value,
	) -> Result<usize, RuntimeError> {
		let fd = match self.net.sockets.get(&sid) {
			Some(e) => e.raw_fd(),
			None => {
				return Err(RuntimeError::new(format!(
					"net: park on unknown socket {sid}"
				)));
			}
		};
		if self.net.poller.is_none() {
			self.net.poller =
				Some(Poller::new().map_err(|e| RuntimeError::new(format!("net: poller: {e}")))?);
		}
		let token = self.net.next_token;
		self.net.next_token += 1;
		let ev = match interest {
			Interest::Read => Event::readable(token),
			Interest::Write => Event::writable(token),
		};
		// SAFETY: the socket lives in `self.net.sockets` and is removed from the
		// poller (`delete`) on wake or reap, before it can be closed/dropped. Each
		// socket is owned by one fiber at a time, so an fd is never double-added.
		unsafe {
			self
				.net
				.poller
				.as_ref()
				.unwrap()
				.add(fd, ev)
				.map_err(|e| RuntimeError::new(format!("net: poller add: {e}")))?;
		}
		self.net.waits.insert(token, IoWait { fid, fd, retry });
		Ok(token)
	}

	// Drop a parked I/O wait (on cancellation / reaping). Idempotent.
	pub(crate) fn reactor_deregister(&mut self, token: usize) {
		if let Some(w) = self.net.waits.remove(&token) {
			if let Some(p) = &self.net.poller {
				// SAFETY: same fd we added; deleted before the socket is dropped.
				let _ = p.delete(unsafe { BorrowedFd::borrow_raw(w.fd) });
			}
		}
	}

	// Block until at least one parked socket is ready (or `timeout` elapses),
	// returning the `(fid, retry)` pairs to re-ready. Called only when there are
	// live I/O waits.
	pub(crate) fn reactor_poll(
		&mut self,
		timeout: Option<Duration>,
	) -> Result<Vec<(usize, Value)>, RuntimeError> {
		let ready: Vec<usize> = {
			let NetState { poller, events, .. } = &mut self.net;
			let poller = poller.as_mut().expect("poller exists when waits non-empty");
			events.clear();
			poller
				.wait(events, timeout)
				.map_err(|e| RuntimeError::new(format!("net: poll: {e}")))?;
			events.iter().map(|e| e.key).collect()
		};
		let mut woken = Vec::with_capacity(ready.len());
		for token in ready {
			if let Some(w) = self.net.waits.remove(&token) {
				if let Some(p) = &self.net.poller {
					// SAFETY: same fd we added; deleted before the socket is dropped.
					let _ = p.delete(unsafe { BorrowedFd::borrow_raw(w.fd) });
				}
				woken.push((w.fid, w.retry));
			}
		}
		Ok(woken)
	}
}
