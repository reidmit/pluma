// The shared blocking-I/O offload subsystem (notes/IO.md): a `Reactor` unifying socket
// *readiness* (`std.sys.net`) and *completion* of non-pollable blocking work — file I/O,
// SQLite, name resolution — under one `polling::Poller`, plus the `BlockingPool` of
// worker threads that run those blocking calls off the single scheduler thread.
//
// The scheduler thread is the only one that touches wasm/V8; a worker only ever runs a
// blocking syscall and hands back the bytes. A worker signals completion by pushing its
// result onto a shared queue and poking the poller (`Poller::notify`), which the
// scheduler's `io-poll` step drains alongside socket-readiness events — one poll step,
// two wake sources. So the park/wake/settle machinery is written once and every client
// (net, fs, db, …) reuses it; only *which* thread runs the job differs (a general pool
// for stateless ops; a pinned worker per stateful resource like a SQLite connection).
//
// Engine-independent: the V8 callbacks in `v8host` shape these results into the
// marshalling ABI, but nothing here touches V8.

use std::collections::{HashMap, HashSet, VecDeque};
use std::os::fd::{BorrowedFd, RawFd};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polling::{Event, Events, Poller};

/// libuv-style default worker count for the general pool (stateless file/DNS/crypto ops).
const POOL_SIZE: usize = 4;

/// The outcome of one offloaded blocking op, handed back to the parked fiber by the op's
/// host callback (which marshals it into a `result`). Mirrors `net::NetRet`'s value arms;
/// `Nothing`/`Count`/`Bytes` are the ok shapes, `Err` carries the OS message.
// `Count` (a bytes-written / rows-affected channel) lands with a later client; async fs
// uses `Bytes`/`Nothing`/`Err`, the sleep op only `Nothing`.
#[allow(dead_code)]
pub(crate) enum OpResult {
	Bytes(Vec<u8>),
	Count(i64),
	Nothing,
	/// A connected socket from an offloaded `net.connect` (the worker did the blocking DNS +
	/// handshake); the scheduler thread adopts it into `HostNet` on collect.
	Conn(std::net::TcpStream),
	Err(String),
}

/// A unit of blocking work for a pool worker, paired with the fiber that submitted it so
/// the completion can be routed back. The closure runs off the scheduler thread.
type Submission = (i32, Box<dyn FnOnce() -> OpResult + Send>);

/// Read- vs write-readiness for a socket park. The reactor owns the poller registration,
/// so this lives here; `net` passes it through when a socket op would block.
#[derive(Clone, Copy)]
pub(crate) enum Interest {
	Read,
	Write,
}

/// The shared readiness + completion reactor. Owns the one `polling::Poller` that both
/// socket readiness (`HostNet`) and offload completions feed into, plus the worker pool
/// the offload clients submit to. Lives in `HostState` so it persists for the whole run.
pub(crate) struct Reactor {
	/// The one readiness+completion poller, shared (`Arc`) with the pool's workers so they
	/// can `notify()` it from their thread. Created eagerly — one kqueue/epoll fd per run
	/// is negligible, and it lets workers capture the handle when the pool spawns.
	poller: Arc<Poller>,
	events: Events,
	/// Socket readiness waits: fiber id (the poller token) → the fd to deregister on wake.
	waits: HashMap<i32, RawFd>,
	/// Woken fibers (socket-ready or offload-complete), buffered across `poll` calls — one
	/// `wait` can surface several; the scheduler consumes one fid per `poll`.
	ready: VecDeque<i32>,
	/// Completed offload results pushed by workers, drained into `done` on `poll`.
	completions: Arc<Mutex<VecDeque<(i32, OpResult)>>>,
	/// Drained completion results keyed by fiber id, awaiting the fiber's *collect* call
	/// (the op's host callback's second invocation, after the wake re-runs the parked task).
	done: HashMap<i32, OpResult>,
	/// Offload fibers whose op is submitted but not yet drained. Bounds `poll`'s
	/// block-forever guard and scopes `discarded` to genuinely in-flight ops.
	inflight: HashSet<i32>,
	/// Fibers whose offload op was cancelled (scope reaped) while still in flight: drop the
	/// worker's result on arrival instead of stashing it in `done`. (notes/IO.md cancellation.)
	discarded: HashSet<i32>,
	/// The worker pool's job sender, spawned lazily on the first `submit` so a program that
	/// never offloads spawns no threads.
	pool: Option<Sender<Submission>>,
}

impl Default for Reactor {
	fn default() -> Self {
		Reactor {
			poller: Arc::new(Poller::new().expect("create I/O poller")),
			events: Events::new(),
			waits: HashMap::new(),
			ready: VecDeque::new(),
			completions: Arc::new(Mutex::new(VecDeque::new())),
			done: HashMap::new(),
			inflight: HashSet::new(),
			discarded: HashSet::new(),
			pool: None,
		}
	}
}

impl Reactor {
	/// Register fiber `fid` against `fd`'s readiness (token = fid), reporting whether the
	/// poller accepted it. The socket lives in `HostNet::sockets`; net deregisters the fd
	/// (on wake / unwatch) before it can be closed.
	pub(crate) fn register_socket(
		&mut self,
		fid: i32,
		fd: RawFd,
		interest: Interest,
	) -> Result<(), String> {
		let ev = match interest {
			Interest::Read => Event::readable(fid as usize),
			Interest::Write => Event::writable(fid as usize),
		};
		// SAFETY: one fiber owns a socket op at a time (so an fd is never double-added), and
		// the fd is deleted on wake / unwatch before the socket is dropped.
		unsafe { self.poller.add(fd, ev) }.map_err(|e| format!("net: poller add: {e}"))?;
		self.waits.insert(fid, fd);
		Ok(())
	}

	/// Submit blocking `job` for fiber `fid` to a worker thread. The fiber then parks on
	/// `wait::IO`; the worker's completion wakes it through `poll`. The pool spawns on the
	/// first call.
	pub(crate) fn submit(&mut self, fid: i32, job: Box<dyn FnOnce() -> OpResult + Send>) {
		if self.pool.is_none() {
			self.pool = Some(spawn_pool(self.poller.clone(), self.completions.clone()));
		}
		self.inflight.insert(fid);
		// Workers loop forever, so the channel never closes mid-run; treat an impossible
		// send failure as a dropped op rather than hanging the fiber on a phantom wake.
		if self.pool.as_ref().unwrap().send((fid, job)).is_err() {
			self.inflight.remove(&fid);
		}
	}

	/// Pull the completed result for `fid` if its worker has finished (the *collect* call
	/// the woken fiber makes when it re-runs its parked op). `None` before completion — the
	/// op then submits and parks.
	pub(crate) fn collect(&mut self, fid: i32) -> Option<OpResult> {
		self.done.remove(&fid)
	}

	/// Block until a parked socket is ready or a worker completion lands (or `deadline`
	/// nanos elapse; `-1` = block indefinitely), returning one woken fid (`-1` on timeout /
	/// nothing pending). Drains both wake sources — socket-readiness events and the worker
	/// completion queue — into the `ready` buffer; extra woken fids surface on later calls.
	pub(crate) fn poll(&mut self, deadline: i64) -> i32 {
		if self.ready.is_empty() {
			// No wake source pending (no parked socket, nothing in flight): don't block
			// forever. (The scheduler only calls us with a `wait::IO` fiber present, so this
			// guards a fiber reaped out from under its only pending op.)
			if self.waits.is_empty() && self.inflight.is_empty() {
				return -1;
			}
			let timeout = if deadline < 0 {
				None
			} else {
				Some(Duration::from_nanos(deadline as u64))
			};
			self.events.clear();
			if self.poller.wait(&mut self.events, timeout).is_err() {
				return -1;
			}
			// Socket readiness: each ready event's token is the parked fid.
			for ev in self.events.iter() {
				let fid = ev.key as i32;
				if let Some(fd) = self.waits.remove(&fid) {
					// SAFETY: same fd we added; deleted before the socket is dropped.
					let _ = self.poller.delete(unsafe { BorrowedFd::borrow_raw(fd) });
					self.ready.push_back(fid);
				}
			}
			// Worker completions (woken by `Poller::notify`): stash each result for its
			// fiber's collect call, unless the op was cancelled mid-flight.
			let mut q = self.completions.lock().unwrap();
			while let Some((fid, res)) = q.pop_front() {
				self.inflight.remove(&fid);
				if self.discarded.remove(&fid) {
					continue; // cancelled — drop the result
				}
				self.done.insert(fid, res);
				self.ready.push_back(fid);
			}
		}
		self.ready.pop_front().unwrap_or(-1)
	}

	/// Drop a parked I/O wait on cancellation / reaping (the `io-unwatch` import).
	/// Idempotent, and uniform over both wake sources: deregister a socket wait, or — for
	/// an in-flight offload op — arrange to drop its result when the worker finishes (or
	/// discard a result that already landed).
	pub(crate) fn unwatch(&mut self, fid: i32) {
		if let Some(fd) = self.waits.remove(&fid) {
			// SAFETY: same fd we added; deleted before the socket is dropped.
			let _ = self.poller.delete(unsafe { BorrowedFd::borrow_raw(fd) });
			return; // a socket wait carries no offload result
		}
		if self.inflight.contains(&fid) {
			self.discarded.insert(fid); // worker still running — drop its result on arrival
		} else {
			self.done.remove(&fid); // result already landed — drop it
		}
	}
}

/// Spawn the general worker pool: `POOL_SIZE` threads draining a shared job channel. Each
/// runs one submission's blocking closure, pushes `(fid, result)` onto `completions`, and
/// `notify`s the poller so the scheduler's next `io-poll` drains it. Threads exit when the
/// `Sender` drops at end of run (the receiver's `recv` returns `Err`).
fn spawn_pool(
	poller: Arc<Poller>,
	completions: Arc<Mutex<VecDeque<(i32, OpResult)>>>,
) -> Sender<Submission> {
	let (tx, rx) = mpsc::channel::<Submission>();
	// std's `Receiver` isn't `Sync`; share one behind a mutex so any free worker can pull
	// the next job (lock only spans the dequeue, released before the blocking call runs).
	let rx = Arc::new(Mutex::new(rx));
	for _ in 0..POOL_SIZE {
		let rx = rx.clone();
		let poller = poller.clone();
		let completions = completions.clone();
		std::thread::spawn(move || {
			loop {
				let job = {
					let guard = rx.lock().unwrap();
					guard.recv()
				};
				let Ok((fid, job)) = job else { break }; // sender dropped → shut down
				let result = job();
				completions.lock().unwrap().push_back((fid, result));
				// Wake the scheduler's `poll`; an error here only means the poller is gone
				// (run tearing down), in which case the result is moot.
				let _ = poller.notify();
			}
		});
	}
	tx
}
