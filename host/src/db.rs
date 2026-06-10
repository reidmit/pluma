// Embedded SQLite for `std.sys.db`, the second real client of the offload subsystem
// (host/src/offload.rs). A `rusqlite::Connection` is not `Sync`, so unlike the stateless
// file ops (which any general-pool worker runs) every db op runs on **one pinned worker
// thread** that owns the connections — which also serializes writes (SQLite does anyway)
// and is the natural transaction boundary. The worker reports completions through the same
// `CompletionSink` the pool uses, so the scheduler's one `poll` step drains db results
// alongside socket readiness and file completions.
//
// `HostDb` is engine-independent (no V8); the V8 glue is `v8host/db.rs`. It's also
// rusqlite-agnostic in shape — a later swap to a pure-Rust engine (turso) is confined here.
//
// Values cross the wasm boundary as a small length-prefixed binary blob (the same format
// `std.sys.db` encodes/decodes on the Pluma side): a tag byte per value, ints/floats as
// decimal text terminated by `\n`, strings/blobs length-prefixed (`<len>\n<bytes>`). Rows
// come back as `<row-count>\n` then per row `<col-count>\n` then per column a length-
// prefixed name followed by a tagged value.

use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};

use rusqlite::Connection;
use rusqlite::types::{Value as SqliteValue, ValueRef};

use crate::offload::{CompletionSink, OpResult};

/// One cell value — SQLite's five storage classes, mirroring Pluma's `sql.value`.
enum Cell {
	Null,
	Int(i64),
	Real(f64),
	Text(String),
	Blob(Vec<u8>),
}

/// A job for the pinned db worker. `Open`/`Execute` carry the fiber id so their result can
/// be routed back; `Close` is fire-and-forget teardown (the worker drops the connection
/// when it drains the job), so it has no fiber to wake.
enum DbJob {
	Open {
		fid: i32,
		path: String,
	},
	Execute {
		fid: i32,
		conn: i64,
		sql: String,
		params: Vec<u8>,
	},
	Close {
		fid: i32,
		conn: i64,
	},
}

/// The db host state: a handle to the pinned worker, spawned lazily on the first op so a
/// program that never touches a database spawns no thread. Lives in `HostState`.
#[derive(Default)]
pub(crate) struct HostDb {
	worker: Option<Sender<DbJob>>,
}

impl HostDb {
	/// Open `path` for fiber `fid` on the pinned worker; the ok result is the new
	/// connection id (an `OpResult::Count`). Spawns the worker on first use.
	pub(crate) fn open(&mut self, sink: CompletionSink, fid: i32, path: String) {
		self.send(sink, DbJob::Open { fid, path });
	}

	/// Run `sql` (with `params`, the encoded bind list) against connection `conn` for fiber
	/// `fid`; the ok result is the encoded rows (an `OpResult::Bytes`).
	pub(crate) fn execute(
		&mut self,
		sink: CompletionSink,
		fid: i32,
		conn: i64,
		sql: String,
		params: Vec<u8>,
	) {
		self.send(
			sink,
			DbJob::Execute {
				fid,
				conn,
				sql,
				params,
			},
		);
	}

	/// Tear down `conn` for fiber `fid`: the worker drops the connection when it reaches
	/// this job, then completes with `Nothing` so the awaiting fiber resumes.
	pub(crate) fn close(&mut self, sink: CompletionSink, fid: i32, conn: i64) {
		self.send(sink, DbJob::Close { fid, conn });
	}

	fn send(&mut self, sink: CompletionSink, job: DbJob) {
		if self.worker.is_none() {
			self.worker = Some(spawn_worker(sink));
		}
		// The worker loops forever, so the channel never closes mid-run; an impossible send
		// failure just drops the job (its fiber, if any, is then woken by the reactor's
		// block-forever guard only via cancellation — acceptable for this edge).
		let _ = self.worker.as_ref().unwrap().send(job);
	}
}

/// Spawn the single pinned worker: it owns every open connection and runs every db op
/// serially, reporting each `Open`/`Execute` result back through `sink`. Exits when the
/// `Sender` drops at end of run.
fn spawn_worker(sink: CompletionSink) -> Sender<DbJob> {
	let (tx, rx) = mpsc::channel::<DbJob>();
	std::thread::spawn(move || {
		let mut conns: HashMap<i64, Connection> = HashMap::new();
		let mut next_id: i64 = 1;
		while let Ok(job) = rx.recv() {
			match job {
				DbJob::Open { fid, path } => {
					let res = match Connection::open(&path) {
						Ok(c) => {
							let id = next_id;
							next_id += 1;
							conns.insert(id, c);
							OpResult::Count(id)
						}
						Err(e) => OpResult::Err(format!("db.open: {e}")),
					};
					sink.complete(fid, res);
				}
				DbJob::Execute {
					fid,
					conn,
					sql,
					params,
				} => {
					let res = match conns.get_mut(&conn) {
						Some(c) => run_execute(c, &sql, &params),
						None => OpResult::Err(format!("db.execute: unknown connection {conn}")),
					};
					sink.complete(fid, res);
				}
				DbJob::Close { fid, conn } => {
					conns.remove(&conn);
					sink.complete(fid, OpResult::Nothing);
				}
			}
		}
	});
	tx
}

/// Prepare + run `sql`, binding `params`, and encode the result rows. Stepping the
/// statement executes it, so this drives DML (INSERT/UPDATE/CREATE — no rows) and SELECT
/// uniformly. v1 returns rows only; rows-affected is not surfaced.
fn run_execute(conn: &mut Connection, sql: &str, params: &[u8]) -> OpResult {
	let binds = match decode_values(params) {
		Ok(v) => v,
		Err(e) => return OpResult::Err(format!("db.execute: bad params: {e}")),
	};
	let mut stmt = match conn.prepare(sql) {
		Ok(s) => s,
		Err(e) => return OpResult::Err(format!("db.execute: {e}")),
	};
	// Column names must be captured before `query` borrows the statement.
	let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
	let bound: Vec<SqliteValue> = binds.into_iter().map(cell_to_sqlite).collect();
	let mut rows = match stmt.query(rusqlite::params_from_iter(bound)) {
		Ok(r) => r,
		Err(e) => return OpResult::Err(format!("db.execute: {e}")),
	};

	let mut out: Vec<Vec<(String, Cell)>> = Vec::new();
	loop {
		match rows.next() {
			Ok(Some(row)) => {
				let mut cells = Vec::with_capacity(cols.len());
				for (i, name) in cols.iter().enumerate() {
					match row.get_ref(i) {
						Ok(vr) => cells.push((name.clone(), value_ref_to_cell(vr))),
						Err(e) => return OpResult::Err(format!("db.execute: {e}")),
					}
				}
				out.push(cells);
			}
			Ok(None) => break,
			Err(e) => return OpResult::Err(format!("db.execute: {e}")),
		}
	}
	OpResult::Bytes(encode_rows(&out))
}

fn cell_to_sqlite(c: Cell) -> SqliteValue {
	match c {
		Cell::Null => SqliteValue::Null,
		Cell::Int(n) => SqliteValue::Integer(n),
		Cell::Real(f) => SqliteValue::Real(f),
		Cell::Text(s) => SqliteValue::Text(s),
		Cell::Blob(b) => SqliteValue::Blob(b),
	}
}

fn value_ref_to_cell(v: ValueRef<'_>) -> Cell {
	match v {
		ValueRef::Null => Cell::Null,
		ValueRef::Integer(n) => Cell::Int(n),
		ValueRef::Real(f) => Cell::Real(f),
		ValueRef::Text(t) => Cell::Text(String::from_utf8_lossy(t).into_owned()),
		ValueRef::Blob(b) => Cell::Blob(b.to_vec()),
	}
}

// --- the wire codec (mirrors the Pluma side in std/sys/db.pa) ------------------

/// Format an `f64` so it round-trips through Pluma's `string.to-float`: shortest decimal
/// (Rust's `Display`), but guaranteed to carry a `.` or exponent so it never reads back as
/// an integer literal. Non-finite values can't occur from SQLite REAL in practice.
fn fmt_f64(f: f64) -> String {
	let s = format!("{f}");
	if s
		.bytes()
		.any(|b| matches!(b, b'.' | b'e' | b'E' | b'n' | b'N'))
	{
		s
	} else {
		format!("{s}.0")
	}
}

fn push_len(out: &mut Vec<u8>, n: usize) {
	out.extend_from_slice(n.to_string().as_bytes());
	out.push(b'\n');
}

fn encode_cell(out: &mut Vec<u8>, c: &Cell) {
	match c {
		Cell::Null => out.push(0),
		Cell::Int(n) => {
			out.push(1);
			out.extend_from_slice(n.to_string().as_bytes());
			out.push(b'\n');
		}
		Cell::Real(f) => {
			out.push(2);
			out.extend_from_slice(fmt_f64(*f).as_bytes());
			out.push(b'\n');
		}
		Cell::Text(s) => {
			out.push(3);
			push_len(out, s.len());
			out.extend_from_slice(s.as_bytes());
		}
		Cell::Blob(b) => {
			out.push(4);
			push_len(out, b.len());
			out.extend_from_slice(b);
		}
	}
}

fn encode_rows(rows: &[Vec<(String, Cell)>]) -> Vec<u8> {
	let mut out = Vec::new();
	push_len(&mut out, rows.len());
	for row in rows {
		push_len(&mut out, row.len());
		for (name, val) in row {
			push_len(&mut out, name.len());
			out.extend_from_slice(name.as_bytes());
			encode_cell(&mut out, val);
		}
	}
	out
}

/// A cursor over the param blob. The blob is trusted (the Pluma side produced it), so
/// errors here are "shouldn't happen" guards, surfaced as a db error rather than a panic.
struct Cursor<'a> {
	buf: &'a [u8],
	pos: usize,
}

impl<'a> Cursor<'a> {
	fn byte(&mut self) -> Result<u8, String> {
		let b = *self.buf.get(self.pos).ok_or("unexpected end of blob")?;
		self.pos += 1;
		Ok(b)
	}

	/// Read up to (and consuming) the next `\n`, as a string.
	fn line(&mut self) -> Result<String, String> {
		let start = self.pos;
		while self.pos < self.buf.len() && self.buf[self.pos] != b'\n' {
			self.pos += 1;
		}
		if self.pos >= self.buf.len() {
			return Err("unterminated field".into());
		}
		let s = std::str::from_utf8(&self.buf[start..self.pos])
			.map_err(|_| "non-utf8 number")?
			.to_string();
		self.pos += 1; // consume the '\n'
		Ok(s)
	}

	fn len(&mut self) -> Result<usize, String> {
		self.line()?.parse::<usize>().map_err(|e| e.to_string())
	}

	fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
		let end = self.pos.checked_add(n).ok_or("length overflow")?;
		let slice = self.buf.get(self.pos..end).ok_or("blob too short")?;
		self.pos = end;
		Ok(slice)
	}
}

fn decode_values(buf: &[u8]) -> Result<Vec<Cell>, String> {
	let mut cur = Cursor { buf, pos: 0 };
	let mut out = Vec::new();
	while cur.pos < buf.len() {
		out.push(decode_cell(&mut cur)?);
	}
	Ok(out)
}

fn decode_cell(cur: &mut Cursor) -> Result<Cell, String> {
	match cur.byte()? {
		0 => Ok(Cell::Null),
		1 => Ok(Cell::Int(
			cur.line()?.parse::<i64>().map_err(|e| e.to_string())?,
		)),
		2 => Ok(Cell::Real(
			cur.line()?.parse::<f64>().map_err(|e| e.to_string())?,
		)),
		3 => {
			let n = cur.len()?;
			let s = std::str::from_utf8(cur.take(n)?)
				.map_err(|_| "non-utf8 text param")?
				.to_string();
			Ok(Cell::Text(s))
		}
		4 => {
			let n = cur.len()?;
			Ok(Cell::Blob(cur.take(n)?.to_vec()))
		}
		t => Err(format!("bad value tag {t}")),
	}
}
