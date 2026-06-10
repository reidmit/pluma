// Native import callback for `std.sys.db` (host/src/db.rs): one generic `db-op` covering
// open / execute / close, selected by an op-code, all offloaded to the pinned SQLite
// worker. Shapes a db `OpResult` into the same `(status, len)` + `(dst, cap)` byte ABI the
// async fs ops use. Submit-or-collect like every offload op: the first call reads the args
// out of scratch (only the scheduler thread may touch V8 memory) and submits to the worker,
// reporting would-block; the wake's re-run collects the worker's result and marshals it.

use super::Ctx;
use super::marshal::{argi, ctx_and_mem, deliver_read_v8, read_mem, read_str, set_pair};
use crate::offload::OpResult;

// Op-codes shared with `std/sys/db.pa` (the `op-*` defs there).
const OP_OPEN: i32 = 0;
const OP_EXECUTE: i32 = 1;
const OP_CLOSE: i32 = 2;
const OP_BATCH: i32 = 3;

/// Deliver a db `OpResult` into the caller's `(dst, cap)` buffer + a `(status, len)` pair.
/// `Bytes` is the encoded rows (execute); `Count` is the new connection id (open), handed
/// back as its decimal text for the Pluma side to parse; `Nothing` is a void op (close).
/// `Err` stashes its message in `last_error`. status: 0 ok, 2 error.
fn deliver_db(
	scope: &mut v8::HandleScope,
	mem: v8::Local<v8::Object>,
	ctx: &mut Ctx,
	dst: i32,
	cap: i32,
	res: OpResult,
) -> (i32, i32) {
	match res {
		OpResult::Bytes(b) => (0, deliver_read_v8(scope, mem, ctx, dst, cap, b)),
		OpResult::Count(n) => (
			0,
			deliver_read_v8(scope, mem, ctx, dst, cap, n.to_string().into_bytes()),
		),
		OpResult::Nothing => (0, 0),
		OpResult::Err(e) => {
			ctx.state.last_error = e;
			(2, 0)
		}
		OpResult::Conn(_) => unreachable!("db op produced a Conn"),
	}
}

/// `db-op(i32 fid, i32 op, i64 conn, i32 sql_ptr, i32 sql_len, i32 params_ptr, i32
/// params_len, i32 dst, i32 cap) -> (i32 status, i32 len)`: run an `std.sys.db` op on the
/// pinned worker. For `open`, `sql` is the path and `params` is empty; for `execute`,
/// `conn` selects the connection and `params` is the encoded bind list; the ok payload
/// (rows, or the new connection id as text) comes back through `(dst, cap)` (overflow
/// stashed for `io-copyout`).
pub(super) fn cb_db_op(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let fid = argi(scope, &args, 0);
	let op = argi(scope, &args, 1);
	let conn = argi(scope, &args, 2) as i64;
	let (sp, sl) = (argi(scope, &args, 3), argi(scope, &args, 4));
	let (pp, pl) = (argi(scope, &args, 5), argi(scope, &args, 6));
	let (dst, cap) = (argi(scope, &args, 7), argi(scope, &args, 8));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	match ctx.state.reactor.collect(fid) {
		Some(res) => {
			let (s, n) = deliver_db(scope, mem, ctx, dst, cap, res);
			set_pair(scope, &mut rv, s, n);
		}
		None => {
			let text = read_str(scope, mem, sp, sl);
			let params = read_mem(scope, mem, pp.max(0) as usize, pl.max(0) as usize);
			let sink = ctx.state.reactor.completion_sink();
			ctx.state.reactor.mark_inflight(fid);
			match op {
				OP_OPEN => ctx.state.db.open(sink, fid, text),
				OP_CLOSE => ctx.state.db.close(sink, fid, conn),
				OP_BATCH => ctx.state.db.batch(sink, fid, conn, text),
				OP_EXECUTE => ctx.state.db.execute(sink, fid, conn, text, params),
				_ => ctx.state.db.execute(sink, fid, conn, text, params),
			}
			set_pair(scope, &mut rv, 1, 0); // would-block
		}
	}
}
