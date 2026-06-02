// Value: the runtime representation of all Pluma values. Compound payloads
// live behind `Rc` so cloning a Value is a refcount bump. Pluma is immutable,
// so Rc-sharing is always safe — no copy-on-write needed.
//
// Variant sizes: every payload is one machine word (`Rc<T>`, `i64`, `f64`)
// except `Builtin`, whose `Rc<str>` is a two-word fat pointer (data + length).
// The resulting enum is 24 bytes — slightly larger than the prior 16-byte
// invariant, in exchange for runtime tags being plain strings.
//
// Numeric tag dispatch is the hot path in the eval loop; using a plain enum
// (vs. NaN-boxing or pointer-tagging) keeps this fast on stable Rust without
// unsafe code. See PERF-NOTES for the eventual NaN-boxing direction.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// Persistent (structurally-shared) containers backing `core.dict`. Cloning a
// root is an O(1) refcount bump and insert/remove copy only the O(log n) path
// they touch — so an immutable `dict.insert` no longer deep-copies the whole
// map (which made a build-up loop O(n²)). `im_rc` is the Rc-based (single-
// threaded, no atomics) variant, matching the VM's `Rc` value model.
use im_rc::{HashMap as ImMap, Vector as ImVec};

#[derive(Clone)]
pub enum Value {
	Nothing,
	Bool(bool),
	Int(i64),
	Float(f64),
	String(Rc<String>),
	// Bytes are an immutable, arbitrary-content byte sequence — no UTF-8
	// invariant. Distinct from String at the type level: explicit
	// `string.to-bytes` / `bytes.to-string` is the only bridge.
	Bytes(Rc<Vec<u8>>),
	Tuple(Rc<Vec<Value>>),
	// Lists are stored in an interior-mutable cell. Pluma lists are immutable
	// *by convention* — every list operation builds a fresh list and nothing
	// reads through the cell expecting it to change — with one deliberate
	// escape hatch: `list.set` (`list-set`) overwrites a slot in place. That
	// needs interior mutability because builtin args arrive cloned (so a
	// copy-on-write set would always copy); it also matches how the wasm
	// backend already stores a list (a shared, mutable `$valarray`). Sharing
	// a list value shares the cell, so a `list.set` is visible through every
	// alias — the price of the escape hatch.
	List(Rc<RefCell<Vec<Value>>>),
	Record(Rc<HashMap<String, Value>>),
	Variant(Rc<VariantData>),
	Closure(Rc<ClosureData>),
	// A primitive call dispatched by tag. The tag names an entry in
	// `builtin::call_builtin`'s match — an unknown tag is a runtime
	// error, reached only if a `built-in "..."` in stdlib `.pa` source
	// names a handler builtin.rs doesn't implement. `Rc<str>` keeps the
	// tag owned (no leak) and cheap to clone.
	Builtin(Rc<str>),
	// An opaque wall-clock instant: nanoseconds since the Unix epoch (UTC),
	// signed. The surface type is `instant` (a `core.time` primitive); the
	// VM only ever produces these from `core.time` builtins.
	Instant(i64),
	// An opaque time span in nanoseconds, signed. Surface type `duration`.
	Duration(i64),
	VariantCtor(Rc<VariantCtorData>),
	// A typeclass method dictionary: a positional array of method values,
	// indexed by trait declaration order. Built per-instance at program load
	// (concrete instances) or per-call (parametric instances; phase 3). The VM
	// never inspects a method dict directly — only `GetDictField` reads from
	// one. Distinct from `Dict` below, which is the user-facing `core.dict`.
	MethodDict(Rc<Vec<Value>>),
	// The user-facing `core.dict`: an immutable, insertion-ordered key/value
	// table. Keys live in `entries` in the order they were first inserted;
	// `buckets` indexes them by the caller-supplied hash so lookup is O(1)
	// average. All mutations (insert / remove) return a fresh `DictData`;
	// Rc-sharing keeps that cheap.
	Dict(Rc<DictData>),
	// A mutable cell. Identity-based: two `Ref` values are equal iff they
	// point to the same underlying cell. Aliasing is intentional — passing
	// a ref to a function lets that function observe and mutate the cell.
	Ref(Rc<RefCell<Value>>),
	// A callable that, when applied, builds a *cold* task rather than
	// running. Codegen emits one for any async-bearing function (a function
	// whose body awaits a task via `try`): calling it (see `do_call`)
	// packages the args + captures into a `Task(TaskRepr::Async{..})` recipe.
	// This is the runtime fact that makes "calling an async function returns
	// a cold task" — no call-site knowledge needed. See `vm::task`.
	AsyncFn(Rc<AsyncFnData>),
	// A cold, re-runnable asynchronous computation. Building one does
	// nothing; the driver (`VM::run_task`) runs it when it's awaited (the
	// `Await` op inside a step function) or returned from `main`. Awaiting
	// the same `Task` value twice runs it twice — it's a recipe, not a
	// cached result. See `vm::task`.
	Task(Rc<TaskRepr>),
	// A live scope's handle — the value bound by `scope as s`. It's just the
	// scheduler's id for the scope; the handle methods (`scope-spawn` etc.)
	// read it to find the scope. Created by the driver when it starts a
	// `TaskRepr::Scope`, never by user code. See `vm::task`.
	ScopeHandle(usize),
}

impl Value {
	/// Build a `List` from an owned `Vec`. Wraps it in the interior-mutable
	/// cell every list lives in (see the `List` variant) so construction sites
	/// don't repeat the `Rc::new(RefCell::new(..))` boilerplate.
	pub fn list(items: Vec<Value>) -> Value {
		Value::List(Rc::new(RefCell::new(items)))
	}
}

pub struct AsyncFnData {
	// Index into `Program::functions` of the resumable step function — the
	// async function's body lowered with `Await` suspension points.
	pub step_fn: usize,
	pub captures: Rc<Vec<Value>>,
}

// The shapes a `task` value can take. The leaf variants are produced by the
// `core.task` primitives (`task.return`/`fail`/`sleep`/`yield`); `Async` is
// produced by calling an async-bearing function. The driver in `vm::task`
// interprets them.
pub enum TaskRepr {
	// `task.return v` — already finished, produces `v`.
	Pure(Value),
	// `task.fail e` — fails immediately with `e` (untyped error channel).
	Fail(Value),
	// `task.sleep d` — completes (with nothing) after `d` nanoseconds.
	Sleep(i64),
	// `task.yield ()` — hands the scheduler a turn, then completes.
	Yield,
	// A cold instance of an async function: its resumable step function plus
	// the closure captures and the call arguments to seed the first frame.
	Async {
		step_fn: usize,
		captures: Rc<Vec<Value>>,
		args: Vec<Value>,
	},
	// Combinator nodes (built by the `task.*` builtins). The driver interprets
	// them as activation frames that transform a sub-task's outcome.
	//
	// `task.then t k` — run `t`; on success feed the value to `k` (a
	// `fun a -> task b`) and run that; on failure, propagate (skip `k`).
	Then {
		task: Box<Value>,
		k: Value,
	},
	// `task.or-else t recover` — run `t`; on success pass the value through;
	// on failure run `recover` (a `fun nothing -> task a`) and run its task.
	// The recovering dual of `then`; what `??` desugars to over `task`.
	OrElse {
		task: Box<Value>,
		recover: Value,
	},
	// `task.attempt t` — run `t`; reify the outcome into the value channel:
	// success -> `ok v`, failure -> `err e`. The result task never fails.
	Attempt {
		task: Box<Value>,
	},
	// `task.map f t` — run `t`; apply the pure `f` (a `fun a -> b`) to its
	// value; propagate failure unchanged.
	Map {
		task: Box<Value>,
		f: Value,
	},
	// `task.shielded t` — run `t` in an uninterruptible region: the driver
	// runs it to completion atomically (within one pump), so a concurrent
	// cancellation can't interrupt it — it's only observed once `t` settles.
	// For self-contained critical sections / cleanup. `t` may not await across
	// fibers (a scope handle or `s.next`); that needs the scheduler to
	// interleave, so it's a runtime error inside a shield.
	Shielded {
		task: Box<Value>,
	},
	// A structured-concurrency scope (built by `scope-new`, i.e. what the
	// `scope` keyword lowers to). When the driver runs it, it creates a fresh
	// scope, calls `body_fn` with the scope's handle, and runs the resulting
	// task as the scope's root — blocking the scope's completion until every
	// child spawned into it has settled or been cancelled. `manual` selects
	// the non-fail-fast form. See `vm::task`.
	Scope {
		manual: bool,
		body_fn: Value,
	},
	// A hot handle to a spawned child fiber (returned by `scope-spawn`).
	// Awaiting it waits for fiber `usize` to settle; awaiting again yields the
	// cached result. The `usize` is the scheduler's fiber id.
	Handle(usize),
	// `s.next` on a manual scope (`usize` is the scope id): a task that drains
	// the scope's next settled child — `some (ok v)` / `some (err e)`, then
	// `none` once all children are drained.
	Next(usize),
	// --- core.net socket ops (see `vm::net`). Each attempts a non-blocking
	// syscall when run; on `WouldBlock` the driver parks the fiber on the I/O
	// reactor and re-runs this same task once the socket is ready. The `u32` is
	// a socket id (an index into `NetState::sockets`). ---
	// `net.accept l` — accept a connection on listener `l`.
	NetAccept(u32),
	// `net.read c n` — read up to `n` bytes from connection `c`.
	NetRead(u32, usize),
	// `net.write c bytes` — one write of `bytes` to connection `c`.
	NetWrite(u32, Rc<Vec<u8>>),
}

#[derive(Clone)]
pub struct DictData {
	// Insertion-ordered (key, value) pairs — a persistent vector, so iteration
	// (`keys`/`values`/`entries`) still observes insertion order. Appended on
	// `insert` of a new key; the slot is overwritten when replacing an existing
	// key's value; rebuilt on `remove`.
	pub entries: ImVec<(Value, Value)>,
	// Hash → indices into `entries`, persistent so the spine is shared across
	// versions. Collisions chain by walking the (small) index Vec and checking
	// `values_eq` on the keys.
	pub buckets: ImMap<i64, Vec<usize>>,
}

impl DictData {
	pub fn new() -> Self {
		Self {
			entries: ImVec::new(),
			buckets: ImMap::new(),
		}
	}

	// Returns the index in `entries` of the entry whose key equals `key`
	// at hash `h`, or None if no such entry exists.
	pub fn find_index(&self, h: i64, key: &Value) -> Option<usize> {
		let chain = self.buckets.get(&h)?;
		for &idx in chain {
			if values_eq(&self.entries[idx].0, key) {
				return Some(idx);
			}
		}
		None
	}

	// Insert (or replace) without mutating self. The clone is an O(1) refcount
	// bump of the shared spine; the persistent insert below copies only the
	// O(log n) path it touches — no whole-map deep copy.
	pub fn inserted(&self, h: i64, key: Value, value: Value) -> Self {
		let mut d = self.clone();
		d.insert_in_place(h, key, value);
		d
	}

	// Insert (or replace) mutating self. Cheap because the backing `im_rc`
	// containers are persistent: a `set`/`push_back`/`insert` copies only the
	// path from the root to the touched node and shares everything else with the
	// pre-insert version, so an immutable `dict.insert` loop is O(n log n) total
	// rather than the O(n²) a flat-`Vec` deep-copy-per-insert produced.
	pub fn insert_in_place(&mut self, h: i64, key: Value, value: Value) {
		if let Some(idx) = self.find_index(h, &key) {
			let _ = self.entries.set(idx, (key, value));
		} else {
			let idx = self.entries.len();
			self.entries.push_back((key, value));
			let mut chain = self.buckets.get(&h).cloned().unwrap_or_default();
			chain.push(idx);
			self.buckets.insert(h, chain);
		}
	}

	// Remove without mutating self. Returns a fresh DictData with the entry
	// gone and indices renumbered to stay dense.
	pub fn removed(&self, h: i64, key: &Value) -> Self {
		match self.find_index(h, key) {
			None => self.clone(),
			Some(removed_idx) => {
				let mut entries = ImVec::new();
				for (i, e) in self.entries.iter().enumerate() {
					if i != removed_idx {
						entries.push_back(e.clone());
					}
				}
				// Rebuild the bucket index against the renumbered entries.
				let mut buckets: ImMap<i64, Vec<usize>> = ImMap::new();
				for (h2, idxs) in self.buckets.iter() {
					let mapped: Vec<usize> = idxs
						.iter()
						.filter_map(|&i| {
							if i == removed_idx {
								None
							} else if i > removed_idx {
								Some(i - 1)
							} else {
								Some(i)
							}
						})
						.collect();
					if !mapped.is_empty() {
						buckets.insert(*h2, mapped);
					}
				}
				Self { entries, buckets }
			}
		}
	}
}

pub struct VariantData {
	pub qualified_enum: Rc<String>,
	pub variant: Rc<String>,
	pub payload: Vec<Value>,
}

pub struct ClosureData {
	pub fn_idx: usize,
	// Rc-shared so cloning a closure (which happens on every Call) is just a
	// refcount bump rather than a fresh Vec allocation.
	pub captures: Rc<Vec<Value>>,
}

pub struct VariantCtorData {
	pub qualified_enum: Rc<String>,
	pub variant: Rc<String>,
	pub arity: usize,
}

// Display drives `to-string`. Stays consistent with the language reference (docs site).
impl std::fmt::Display for Value {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Value::Int(n) => write!(f, "{}", n),
			Value::Float(n) => {
				if n.fract() == 0.0 && n.is_finite() {
					write!(f, "{:.1}", n)
				} else {
					write!(f, "{}", n)
				}
			}
			Value::String(s) => write!(f, "{}", s),
			Value::Bytes(b) => {
				// Render bytes in the same single-quote literal form they're
				// written in source: printable ASCII inline, everything else
				// (including '\'' and '\\') as \xNN. Round-trips to a
				// parseable bytes literal.
				write!(f, "'")?;
				for &byte in b.iter() {
					match byte {
						b'\\' => write!(f, "\\\\")?,
						b'\'' => write!(f, "\\'")?,
						0x20..=0x7e => write!(f, "{}", byte as char)?,
						_ => write!(f, "\\x{:02x}", byte)?,
					}
				}
				write!(f, "'")
			}
			Value::Bool(b) => write!(f, "{}", b),
			Value::Nothing => write!(f, "()"),
			Value::Tuple(elems) => {
				write!(f, "(")?;
				for (i, v) in elems.iter().enumerate() {
					if i > 0 {
						write!(f, ", ")?;
					}
					write!(f, "{}", v)?;
				}
				write!(f, ")")
			}
			Value::List(elems) => {
				write!(f, "[")?;
				for (i, v) in elems.borrow().iter().enumerate() {
					if i > 0 {
						write!(f, ", ")?;
					}
					write!(f, "{}", v)?;
				}
				write!(f, "]")
			}
			Value::Record(fields) => {
				write!(f, "{{")?;
				let mut entries: Vec<_> = fields.iter().collect();
				entries.sort_by(|a, b| a.0.cmp(b.0));
				for (i, (k, v)) in entries.iter().enumerate() {
					if i > 0 {
						write!(f, ", ")?;
					}
					write!(f, "{}: {}", k, v)?;
				}
				write!(f, "}}")
			}
			Value::Variant(v) => {
				let bare = v
					.qualified_enum
					.rsplit_once('.')
					.map(|(_, n)| n)
					.unwrap_or(&v.qualified_enum);
				write!(f, "{}.{}", bare, v.variant)?;
				for arg in &v.payload {
					write!(f, " {}", arg)?;
				}
				Ok(())
			}
			Value::Closure(_) => write!(f, "<closure>"),
			Value::Builtin(_) => write!(f, "<builtin>"),
			// Wall-clock instants print as RFC 3339 in UTC (e.g.
			// `2026-05-25T14:30:00Z`); durations print in the same form as a
			// duration literal (e.g. `2d`, `1h30m`, `500ms`).
			Value::Instant(nanos) => match jiff::Timestamp::from_nanosecond(*nanos as i128) {
				Ok(ts) => write!(f, "{}", ts),
				Err(_) => write!(f, "<instant {}ns>", nanos),
			},
			Value::Duration(nanos) => write!(f, "{}", format_duration(*nanos)),
			Value::VariantCtor(c) => {
				let bare = c
					.qualified_enum
					.rsplit_once('.')
					.map(|(_, n)| n)
					.unwrap_or(&c.qualified_enum);
				write!(f, "<ctor {}.{}>", bare, c.variant)
			}
			Value::MethodDict(_) => write!(f, "<dict>"),
			Value::Dict(m) => {
				write!(f, "{{")?;
				for (i, (k, v)) in m.entries.iter().enumerate() {
					if i > 0 {
						write!(f, ", ")?;
					}
					write!(f, "{}: {}", k, v)?;
				}
				write!(f, "}}")
			}
			Value::Ref(cell) => write!(f, "ref {}", cell.borrow()),
			Value::AsyncFn(_) => write!(f, "<async-fn>"),
			Value::Task(_) => write!(f, "<task>"),
			Value::ScopeHandle(_) => write!(f, "<scope>"),
		}
	}
}

// Render a duration (nanoseconds) the way a duration literal is written:
// largest unit first, each unit at most once, no spaces, zero components
// dropped (e.g. `90s` -> "1m30s", two days -> "2d"). Matches both the source
// literal syntax and the formatter's rendering, so `print`ed durations read
// back the same way they were written.
fn format_duration(nanos: i64) -> String {
	if nanos == 0 {
		return "0s".to_string();
	}
	let (sign, mut rem): (&str, u128) = if nanos < 0 {
		("-", (nanos as i128).unsigned_abs())
	} else {
		("", nanos as u128)
	};
	const UNITS: [(u128, &str); 7] = [
		(86_400_000_000_000, "d"),
		(3_600_000_000_000, "h"),
		(60_000_000_000, "m"),
		(1_000_000_000, "s"),
		(1_000_000, "ms"),
		(1_000, "us"),
		(1, "ns"),
	];
	let mut out = String::from(sign);
	for (per, name) in UNITS {
		if rem >= per {
			out.push_str(&(rem / per).to_string());
			out.push_str(name);
			rem %= per;
		}
	}
	out
}

// Structural equality for `==` / `!=` / `contains`. Type system enforces same
/// The hash of a primitive value, matching the `hash` trait's `int-hash` /
/// `float-hash` / `string-hash` / `bytes-hash` / `bool-hash` builtins EXACTLY
/// (those delegate here). `None` for non-primitive values, which hash through
/// Pluma-defined instances instead. Shared so the `wire` codec can rebuild a
/// decoded dict's hash buckets identically to `dict.lookup` without threading
/// the `hash` dictionary — but only for primitive keys, which is why
/// wire-derivation rejects dicts with compound keys.
pub fn primitive_hash(v: &Value) -> Option<i64> {
	match v {
		Value::Int(n) => Some(*n),
		// Reinterpret the float's bit pattern as i64 (stable per value).
		Value::Float(f) => Some(f.to_bits() as i64),
		Value::Bool(b) => Some(if *b { 1 } else { 0 }),
		// FNV-1a over the raw bytes — a defined, portable hash (NOT Rust's
		// unstable `DefaultHasher`) so the WasmGC backend computes byte-identical
		// values in pure wasm. Same algorithm the `wire` fingerprint uses.
		Value::String(s) => Some(fnv1a(s.as_bytes()) as i64),
		Value::Bytes(b) => Some(fnv1a(b.as_slice()) as i64),
		_ => None,
	}
}

/// FNV-1a (64-bit) over a byte slice. Shared definition for `string-hash` /
/// `bytes-hash`; the WasmGC backend reimplements the same two constants.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
	const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
	const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
	bytes
		.iter()
		.fold(FNV_OFFSET, |h, &b| (h ^ b as u64).wrapping_mul(FNV_PRIME))
}

// type on both sides, so we only need to compare like with like. Closures,
// builtins, ctors, and regexes always compare false.
pub fn values_eq(a: &Value, b: &Value) -> bool {
	match (a, b) {
		(Value::Int(x), Value::Int(y)) => x == y,
		(Value::Float(x), Value::Float(y)) => x == y,
		(Value::Bool(x), Value::Bool(y)) => x == y,
		(Value::String(x), Value::String(y)) => x == y,
		(Value::Bytes(x), Value::Bytes(y)) => x == y,
		(Value::Instant(x), Value::Instant(y)) => x == y,
		(Value::Duration(x), Value::Duration(y)) => x == y,
		(Value::Nothing, Value::Nothing) => true,
		(Value::Tuple(xs), Value::Tuple(ys)) => {
			xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(a, b)| values_eq(a, b))
		}
		(Value::List(xs), Value::List(ys)) => {
			let (xs, ys) = (xs.borrow(), ys.borrow());
			xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(a, b)| values_eq(a, b))
		}
		(Value::Record(xs), Value::Record(ys)) => {
			xs.len() == ys.len()
				&& xs
					.iter()
					.all(|(k, v)| ys.get(k).map_or(false, |yv| values_eq(v, yv)))
		}
		(Value::Variant(a), Value::Variant(b)) => {
			a.qualified_enum == b.qualified_enum
				&& a.variant == b.variant
				&& a.payload.len() == b.payload.len()
				&& a
					.payload
					.iter()
					.zip(b.payload.iter())
					.all(|(a, b)| values_eq(a, b))
		}
		// Dict equality is structural and order-independent: same key/value
		// set in either order is the same dict. We walk one side and look
		// each key up in the other via its hash bucket.
		// Refs use reference identity, not structural equality: two cells
		// holding 5 are distinct, but a cell compared with itself is always
		// equal regardless of contents.
		(Value::Ref(a), Value::Ref(b)) => Rc::ptr_eq(a, b),
		(Value::Dict(a), Value::Dict(b)) => {
			if a.entries.len() != b.entries.len() {
				return false;
			}
			for (h, idxs) in a.buckets.iter() {
				for &i in idxs {
					let (k, v) = &a.entries[i];
					match b.find_index(*h, k) {
						Some(j) => {
							if !values_eq(v, &b.entries[j].1) {
								return false;
							}
						}
						None => return false,
					}
				}
			}
			true
		}
		_ => false,
	}
}
