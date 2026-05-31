// The hand-emitted async runtime: the WasmGC analogue of `vm::task`. A cold
// `$task` (built by the task primitives + the async-fn lowering) is driven to
// completion by `__task_drive`, a single-fiber poll loop mirroring
// `vm::task::advance_one`'s Start/Ok/Err focus over an activation stack.
//
// STAGE 1 (single fiber): pure/fail/yield/sleep/then/or-else/attempt/map/async/
// shielded + `defer`-on-failure. Timers are virtual and there is one fiber, so
// `yield`/`sleep` resume immediately and `shielded` runs inline. The structured-
// concurrency layer (fibers/scopes/timers/cancellation) is Stage 2.
//
// A CPS poll fn returns a `__poll` variant: `ready(value[, defers])` (vtag 0) or
// `pending(subtask, state')` (vtag 1) — see `ir::cps`. `__poll_step` calls it and
// reports completion vs suspension; the driver pushes a `Poll` activation on
// suspension and resumes it with the awaited value. Activations are `$variant`s
// tagged with an `act_kind`; the stack is a module-level `$valarray`.

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::Wat;
use crate::runtime::{TaskGlobals, TaskLits, act_kind, task_kind};
use crate::types;

// Focus discriminants for the driver's main loop.
const F_START: i32 = 0;
const F_OK: i32 = 1;
const F_ERR: i32 = 2;

/// `__task_drive(root) -> value`: run a cold `$task` to completion. Mirrors
/// `vm::task::advance_one` — a focus (Start/Ok/Err) threaded through a loop over
/// the global activation stack. On Start it dispatches the task's `kind`; on Ok/Err
/// it settles the value down the activation chain. Returns the success value, or a
/// `result.err(e)` on root failure (the harness turns that into a runtime error).
pub(crate) fn build_task_drive_fn(
	poll_step: u32,
	poll_defers_state: u32,
	act_push: u32,
	arity1: u32,
	g: TaskGlobals,
	lits: TaskLits,
) -> Function {
	let v = types::value_ref();
	let va = types::valarray_ref();
	let mut w = Wat::new(1);
	let root = w.param(0);
	let fkind = w.local(ValType::I32);
	let fval = w.local(v);
	let result = w.local(v);
	let tk = w.local(ValType::I32);
	let tp = w.local(va);
	let a = w.local(v);
	let akind = w.local(ValType::I32);
	let apl = w.local(va);
	let ps = w.local(v);
	let pspl = w.local(va);
	let psk = w.local(ValType::I32);
	let pc = w.local(v);

	// Reset the activation stack and seed the focus with `Start(root)`.
	w.i32(16).array_new_default(types::T_VALARRAY).global_set(g.act);
	w.i32(0).global_set(g.actlen);
	w.i32(F_START).local_set(fkind);
	w.local_get(root).local_set(fval);

	w.block("done", |w| {
		w.loop_("main", |w| {
			// ---- Start: `fval` is a `$task`; dispatch its kind. -------------------
			w.local_get(fkind).i32_eqz();
			w.if_(|w| {
				w.local_get(fval)
					.ref_cast(types::T_TASK)
					.struct_get(types::T_TASK, 1)
					.local_set(tk);
				w.local_get(fval)
					.ref_cast(types::T_TASK)
					.struct_get(types::T_TASK, 2)
					.local_set(tp);

				// pure / fail / yield / sleep — settle directly.
				start_settle(w, tk, task_kind::PURE, F_OK, |w| elem(w, tp, 0), fval, fkind);
				start_settle(w, tk, task_kind::FAIL, F_ERR, |w| elem(w, tp, 0), fval, fkind);
				start_settle(w, tk, task_kind::YIELD, F_OK, push_nothing, fval, fkind);
				start_settle(w, tk, task_kind::SLEEP, F_OK, push_nothing, fval, fkind);

				// then / or-else / attempt / map — push an activation, run the inner.
				start_combinator(w, tk, task_kind::THEN, act_kind::THEN, tp, true, act_push, fval, fkind);
				start_combinator(
					w,
					tk,
					task_kind::ORELSE,
					act_kind::ORELSE,
					tp,
					true,
					act_push,
					fval,
					fkind,
				);
				start_combinator(
					w,
					tk,
					task_kind::ATTEMPT,
					act_kind::ATTEMPT,
					tp,
					false,
					act_push,
					fval,
					fkind,
				);
				start_combinator(w, tk, task_kind::MAP, act_kind::MAP, tp, true, act_push, fval, fkind);

				// async — advance the CPS poll fn one step.
				w.local_get(tk).i32(task_kind::ASYNC).i32_eq();
				w.if_(|w| {
					elem(w, tp, 0);
					w.local_set(pc); // poll closure
					w.local_get(pc); // env
					elem(w, tp, 1); // state
					push_nothing(w); // first resume value
					w.call(poll_step).local_set(ps);
					poll_after(w, pc, ps, pspl, psk, fval, fkind, act_push);
				});

				// shielded — single fiber: run the inner inline.
				start_settle(
					w,
					tk,
					task_kind::SHIELDED,
					F_START,
					|w| elem(w, tp, 0),
					fval,
					fkind,
				);

				// scope/handle/next are Stage 2.
				w.unreachable();
			});

			// ---- Ok: settle a value down the activation chain. -------------------
			w.local_get(fkind).i32(F_OK).i32_eq();
			w.if_(|w| {
				w.loop_("ok", |w| {
					w.global_get(g.actlen).i32_eqz();
					w.if_(|w| {
						w.local_get(fval).local_set(result);
						w.br("done");
					});
					pop_activation(w, g, a, akind, apl);

					// poll — resume the suspended poll fn with `fval`.
					w.local_get(akind).i32(act_kind::POLL).i32_eq();
					w.if_(|w| {
						elem(w, apl, 0);
						w.local_set(pc);
						w.local_get(pc);
						elem(w, apl, 1); // state
						w.local_get(fval); // resume value
						w.call(poll_step).local_set(ps);
						poll_after(w, pc, ps, pspl, psk, fval, fkind, act_push);
					});
					// then — run the continuation `k fval`, await its task.
					w.local_get(akind).i32(act_kind::THEN).i32_eq();
					w.if_(|w| {
						call1(w, |w| elem(w, apl, 0), |w| {
						w.local_get(fval);
					}, arity1);
						w.local_set(fval);
						w.i32(F_START).local_set(fkind);
						w.br("main");
					});
					// or-else — success skips the recovery; keep popping.
					w.local_get(akind).i32(act_kind::ORELSE).i32_eq();
					w.if_(|w| {
						w.br("ok");
					});
					// attempt — reify success as `ok fval`; keep popping.
					w.local_get(akind).i32(act_kind::ATTEMPT).i32_eq();
					w.if_(|w| {
						push_result(w, lits.ok_tag, lits.ok_name, |w| {
						w.local_get(fval);
					});
						w.local_set(fval);
						w.br("ok");
					});
					// map — apply `f fval`; keep popping.
					w.local_get(akind).i32(act_kind::MAP).i32_eq();
					w.if_(|w| {
						call1(w, |w| elem(w, apl, 0), |w| {
						w.local_get(fval);
					}, arity1);
						w.local_set(fval);
						w.br("ok");
					});
					w.unreachable();
				});
			});

			// ---- Err: propagate a failure down the activation chain. -------------
			w.local_get(fkind).i32(F_ERR).i32_eq();
			w.if_(|w| {
				w.loop_("err", |w| {
					w.global_get(g.actlen).i32_eqz();
					w.if_(|w| {
						push_result(w, lits.err_tag, lits.err_name, |w| {
						w.local_get(fval);
					});
						w.local_set(result);
						w.br("done");
					});
					pop_activation(w, g, a, akind, apl);

					// poll — the awaiting fn fails too: run its `defer`s, keep propagating.
					w.local_get(akind).i32(act_kind::POLL).i32_eq();
					w.if_(|w| {
						elem(w, apl, 1); // state
						w.call(poll_defers_state).drop();
						w.br("err");
					});
					// then / map — skipped on failure.
					w.local_get(akind).i32(act_kind::THEN).i32_eq();
					w.if_(|w| {
						w.br("err");
					});
					w.local_get(akind).i32(act_kind::MAP).i32_eq();
					w.if_(|w| {
						w.br("err");
					});
					// or-else — recover: run `recover ()` and await its task.
					w.local_get(akind).i32(act_kind::ORELSE).i32_eq();
					w.if_(|w| {
						call1(w, |w| elem(w, apl, 0), push_nothing, arity1);
						w.local_set(fval);
						w.i32(F_START).local_set(fkind);
						w.br("main");
					});
					// attempt — reify failure as `err fval`, then continue as success.
					w.local_get(akind).i32(act_kind::ATTEMPT).i32_eq();
					w.if_(|w| {
						push_result(w, lits.err_tag, lits.err_name, |w| {
						w.local_get(fval);
					});
						w.local_set(fval);
						w.i32(F_OK).local_set(fkind);
						w.br("main");
					});
					w.unreachable();
				});
			});

			w.unreachable();
		});
	});
	w.local_get(result);
	w.finish()
}

/// `__poll_step(pc, state, resume) -> $tuple(kind, x, y)`: call the poll closure
/// once and interpret its `__poll`. `kind` 0 = complete (`x` = tail task), 1 =
/// pending (`x` = sub-task, `y` = next state). A `ready(value, defers)` runs its
/// completion cleanups LIFO before reporting complete.
pub(crate) fn build_poll_step_fn(poll_defers_list: u32, arity2: u32) -> Function {
	let v = types::value_ref();
	let va = types::valarray_ref();
	let mut w = Wat::new(3);
	let (pc, state, resume) = (w.param(0), w.param(1), w.param(2));
	let r = w.local(v);
	let rpl = w.local(va);

	// r = pc(state, resume).
	w.local_get(pc).ref_cast(types::T_CLOSURE);
	w.local_get(state);
	w.local_get(resume);
	w.local_get(pc)
		.ref_cast(types::T_CLOSURE)
		.struct_get(types::T_CLOSURE, 1);
	w.call_indirect(arity2);
	w.local_set(r);

	w.local_get(r)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 3)
		.local_set(rpl);
	// vtag == 0 (ready) ? complete : pending.
	w.local_get(r)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 1)
		.i32_eqz();
	w.if_result(
		v,
		|w| {
			// ready: run completion defers if carried, then tuple(0, value, ()).
			w.local_get(rpl).array_len().i32(2).i32_ge_s();
			w.if_(|w| {
				elem(w, rpl, 1);
				w.call(poll_defers_list).drop();
			});
			push_tuple3(w, 0, |w| elem(w, rpl, 0), push_nothing);
		},
		|w| {
			// pending: tuple(1, subtask, next-state).
			push_tuple3(w, 1, |w| elem(w, rpl, 0), |w| elem(w, rpl, 1));
		},
	);
	w.finish()
}

/// `__poll_defers_list(list) -> nothing`: run a `$list` of zero-arg cleanup
/// closures LIFO. The CPS pass appends, so walk back to front. Each thunk is a
/// `fun { … }` (phantom-unit param, wasm arity 1).
pub(crate) fn build_poll_defers_list_fn(arity1: u32) -> Function {
	let v = types::value_ref();
	let mut w = Wat::new(1);
	let list = w.param(0);
	let arr = w.local(types::valarray_ref());
	let i = w.local(ValType::I32);
	let c = w.local(v);

	w.local_get(list)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(arr);
	w.local_get(arr).array_len().i32(1).i32_sub().local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).i32(0).i32_lt_s().br_if("brk");
			w.local_get(arr)
				.local_get(i)
				.array_get(types::T_VALARRAY)
				.local_set(c);
			call1(w, |w| { w.local_get(c); }, push_nothing, arity1);
			w.drop();
			w.local_get(i).i32(1).i32_sub().local_set(i);
			w.br("lp");
		});
	});
	push_nothing(&mut w);
	w.finish()
}

/// `__poll_defers_state(state) -> nothing`: run the `__defers` cleanup list
/// carried in a suspended poll state, if present. Defer-free poll fns have no such
/// field, so scan the record's names and no-op when it's absent.
pub(crate) fn build_poll_defers_state_fn(eq: u32, poll_defers_list: u32, defers_name: (u32, u32)) -> Function {
	let v = types::value_ref();
	let va = types::valarray_ref();
	let mut w = Wat::new(1);
	let state = w.param(0);
	let names = w.local(va);
	let vals = w.local(va);
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let key = w.local(v);

	str_lit(&mut w, defers_name);
	w.local_set(key);
	w.local_get(state)
		.ref_cast(types::T_RECORD)
		.struct_get(types::T_RECORD, 1)
		.local_set(names);
	w.local_get(state)
		.ref_cast(types::T_RECORD)
		.struct_get(types::T_RECORD, 2)
		.local_set(vals);
	w.local_get(names).array_len().local_set(n);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			// __eq(names[i], "__defers") ?
			w.local_get(names).local_get(i).array_get(types::T_VALARRAY);
			w.local_get(key);
			w.call(eq);
			w.if_(|w| {
				w.local_get(vals).local_get(i).array_get(types::T_VALARRAY);
				w.call(poll_defers_list).drop();
				w.br("brk");
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	push_nothing(&mut w);
	w.finish()
}

/// `__act_push(activation) -> nothing`: push one activation `$value` onto the
/// driver's global stack, growing the backing `$valarray` (doubling) when full.
pub(crate) fn build_act_push_fn(g: TaskGlobals) -> Function {
	let mut w = Wat::new(1);
	let act = w.param(0);
	let cap = w.local(ValType::I32);
	let na = w.local(types::valarray_ref());

	w.global_get(g.act).array_len().local_set(cap);
	w.global_get(g.actlen).local_get(cap).i32_ge_s();
	w.if_(|w| {
		// na = new $valarray(cap == 0 ? 16 : cap * 2).
		w.local_get(cap).i32_eqz();
		w.if_result(
			ValType::I32,
			|w| {
				w.i32(16);
			},
			|w| {
				w.local_get(cap).i32(2).i32_mul();
			},
		);
		w.array_new_default(types::T_VALARRAY).local_set(na);
		w.local_get(na)
			.i32(0)
			.global_get(g.act)
			.i32(0)
			.local_get(cap)
			.array_copy(types::T_VALARRAY, types::T_VALARRAY);
		w.local_get(na).global_set(g.act);
	});
	w.global_get(g.act)
		.global_get(g.actlen)
		.local_get(act)
		.array_set(types::T_VALARRAY);
	w.global_get(g.actlen).i32(1).i32_add().global_set(g.actlen);
	push_nothing(&mut w);
	w.finish()
}

/// `__task_entry(env) -> value`: the async program entry — call the real IR entry
/// (`main`), then drive the cold task it returns. Exported as `_entry` when async.
pub(crate) fn build_task_entry_fn(entry_idx: u32, task_drive: u32) -> Function {
	let mut w = Wat::new(1);
	let env = w.param(0);
	w.local_get(env).call(entry_idx).call(task_drive);
	w.finish()
}

// --------------------------------------------------------------------------
// Shared emission fragments.
// --------------------------------------------------------------------------

/// Push the `i`-th element of a `$valarray` local.
fn elem(w: &mut Wat, arr: crate::helpers::wat::Local, i: i32) {
	w.local_get(arr).i32(i).array_get(types::T_VALARRAY);
}

/// Push the unit `nothing` value.
fn push_nothing(w: &mut Wat) {
	w.i32(types::TAG_NOTHING).struct_new(types::T_VALUE);
}

/// A Start arm that settles directly: `if tk == kind { fval = <val>; fkind =
/// next; goto main }`.
fn start_settle(
	w: &mut Wat,
	tk: crate::helpers::wat::Local,
	kind: i32,
	next: i32,
	val: impl FnOnce(&mut Wat),
	fval: crate::helpers::wat::Local,
	fkind: crate::helpers::wat::Local,
) {
	w.local_get(tk).i32(kind).i32_eq();
	w.if_(|w| {
		val(w);
		w.local_set(fval);
		w.i32(next).local_set(fkind);
		w.br("main");
	});
}

/// A Start arm for a combinator: push an `act_kind` activation (payload `tp[1]`
/// when `has_arg`, else `()`), then run the inner task `tp[0]`.
#[allow(clippy::too_many_arguments)]
fn start_combinator(
	w: &mut Wat,
	tk: crate::helpers::wat::Local,
	kind: i32,
	akind: i32,
	tp: crate::helpers::wat::Local,
	has_arg: bool,
	act_push: u32,
	fval: crate::helpers::wat::Local,
	fkind: crate::helpers::wat::Local,
) {
	w.local_get(tk).i32(kind).i32_eq();
	w.if_(|w| {
		if has_arg {
			push_activation(w, akind, |w| elem(w, tp, 1), push_nothing);
		} else {
			push_activation(w, akind, push_nothing, push_nothing);
		}
		w.call(act_push).drop();
		elem(w, tp, 0);
		w.local_set(fval);
		w.i32(F_START).local_set(fkind);
		w.br("main");
	});
}

/// After `__poll_step`: `ps` is `(kind, x, y)`. On complete, start the tail task
/// `x`; on pending, push a `Poll(pc, y)` activation and start the sub-task `x`.
#[allow(clippy::too_many_arguments)]
fn poll_after(
	w: &mut Wat,
	pc: crate::helpers::wat::Local,
	ps: crate::helpers::wat::Local,
	pspl: crate::helpers::wat::Local,
	psk: crate::helpers::wat::Local,
	fval: crate::helpers::wat::Local,
	fkind: crate::helpers::wat::Local,
	act_push: u32,
) {
	w.local_get(ps)
		.ref_cast(types::T_TUPLE)
		.struct_get(types::T_TUPLE, 1)
		.local_set(pspl);
	// kind = unbox pspl[0].
	w.local_get(pspl)
		.i32(0)
		.array_get(types::T_VALARRAY)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(psk);
	w.local_get(psk).i32_eqz();
	w.if_else(
		|w| {
			// complete: start the tail task.
			elem(w, pspl, 1);
			w.local_set(fval);
			w.i32(F_START).local_set(fkind);
			w.br("main");
		},
		|w| {
			// pending: push Poll(pc, next-state), start the sub-task.
			push_activation(w, act_kind::POLL, |w| { w.local_get(pc); }, |w| elem(w, pspl, 2));
			w.call(act_push).drop();
			elem(w, pspl, 1);
			w.local_set(fval);
			w.i32(F_START).local_set(fkind);
			w.br("main");
		},
	);
}

/// Pop the top activation off the global stack into `(a, akind, apl)`.
fn pop_activation(
	w: &mut Wat,
	g: TaskGlobals,
	a: crate::helpers::wat::Local,
	akind: crate::helpers::wat::Local,
	apl: crate::helpers::wat::Local,
) {
	w.global_get(g.actlen).i32(1).i32_sub().global_set(g.actlen);
	w.global_get(g.act)
		.global_get(g.actlen)
		.array_get(types::T_VALARRAY)
		.local_set(a);
	w.local_get(a)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 1)
		.local_set(akind);
	w.local_get(a)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 3)
		.local_set(apl);
}

/// Push an activation `$variant` `{vtag: kind, payload: [x, y]}` (name unused).
fn push_activation(w: &mut Wat, kind: i32, x: impl FnOnce(&mut Wat), y: impl FnOnce(&mut Wat)) {
	w.i32(types::TAG_VARIANT);
	w.i32(kind);
	w.ref_null(types::T_VALUE);
	x(w);
	y(w);
	w.array_new_fixed(types::T_VALARRAY, 2);
	w.struct_new(types::T_VARIANT);
}

/// Push a 3-tuple `(box kind, x, y)` — the `__poll_step` result shape.
fn push_tuple3(w: &mut Wat, kind: i64, x: impl FnOnce(&mut Wat), y: impl FnOnce(&mut Wat)) {
	w.i32(types::TAG_TUPLE);
	w.i32(types::TAG_INT).i64(kind).struct_new(types::T_INT);
	x(w);
	y(w);
	w.array_new_fixed(types::T_VALARRAY, 3);
	w.struct_new(types::T_TUPLE);
}

/// Push a `result` `$variant` `{vtag: tag, name, payload: [<value>]}`.
fn push_result(w: &mut Wat, tag: u32, name: (u32, u32), val: impl FnOnce(&mut Wat)) {
	w.i32(types::TAG_VARIANT);
	w.i32(tag as i32);
	str_lit(w, name);
	val(w);
	w.array_new_fixed(types::T_VALARRAY, 1);
	w.struct_new(types::T_VARIANT);
}

/// Call a 1-arg closure: `env = clo`, then the arg, then `call_indirect` through
/// its `fn_index`. Leaves the result on the stack.
fn call1(w: &mut Wat, clo: impl Fn(&mut Wat), arg: impl FnOnce(&mut Wat), arity1: u32) {
	clo(w);
	w.ref_cast(types::T_CLOSURE);
	arg(w);
	clo(w);
	w.ref_cast(types::T_CLOSURE)
		.struct_get(types::T_CLOSURE, 1);
	w.call_indirect(arity1);
}

/// Push a fresh `$str` for an interned data-segment literal `(off, len)`.
fn str_lit(w: &mut Wat, (off, len): (u32, u32)) {
	w.i32(types::TAG_STR);
	w.i32(off as i32);
	w.i32(len as i32);
	w.array_new_data(types::T_BYTES, 0);
	w.struct_new(types::T_STR);
}
