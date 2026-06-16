// List helpers: `...rest` tail, tabulating builders, and value-array concat
// (`__list_tail`, `__list_build`, `__list_collect`, `__arrconcat`).

use crate::helpers::dict::{build_none, finish_some, start_some};
use crate::helpers::wat::{Local, Wat};
use crate::runtime::OptionLits;
use crate::types;
use wasm_encoder::{Function, ValType};

/// Build a `$list` value from the value-array in `arr`, leaving it on the stack.
/// The logical `length` field is set to the array's capacity — the single
/// constructor for normal lists; only `list.push` later makes length < capacity.
pub(crate) fn mk_list(w: &mut Wat, arr: Local) {
	w.i32(types::TAG_LIST);
	w.local_get(arr);
	w.local_get(arr).array_len();
	w.struct_new(types::T_LIST);
}

/// Push a `$list`'s logical length (field 2) onto the stack. Use this, NOT
/// `array.len(elems)` (the capacity), wherever a list's element count is needed.
pub(crate) fn list_len(w: &mut Wat, list: Local) {
	w.local_get(list)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 2);
}

/// Build `__list_tail(list, n) -> list`: a fresh list of the elements from index
/// `n` (the `...rest` of a list pattern). `n` is a boxed int.
pub(crate) fn build_list_tail_fn() -> Function {
	let mut w = Wat::new(2);
	let (list, narg) = (w.param(0), w.param(1));
	let src = w.local(types::valarray_ref());
	let dst = w.local(types::valarray_ref());
	let len = w.local(ValType::I32);
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);

	// src = list.elems; len = list.length (logical count, not capacity).
	w.local_get(list)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(src);
	list_len(&mut w, list);
	w.local_set(len);
	// n = (int) narg.
	w.local_get(narg).unbox_int().i32_wrap_i64().local_set(n);
	// dst = new valarray(len - n).
	w.local_get(len)
		.local_get(n)
		.i32_sub()
		.array_new_default(types::T_VALARRAY)
		.local_set(dst);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			// i >= len - n -> done.
			w.local_get(i)
				.local_get(len)
				.local_get(n)
				.i32_sub()
				.i32_ge_s()
				.br_if("brk");
			// dst[i] = src[n + i].
			w.local_get(dst).local_get(i);
			w.local_get(src)
				.local_get(n)
				.local_get(i)
				.i32_add()
				.array_get(types::T_VALARRAY);
			w.array_set(types::T_VALARRAY);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	mk_list(&mut w, dst);
	w.finish()
}

/// Build `__list_push(list, x) -> nothing`: append `x` to `list` in place,
/// amortized O(1). Writes `x` at `length` (growing/swapping the backing array by
/// doubling when full) and bumps `length`. Mutates the `$list` struct's `elems`
/// (field 1) and `length` (field 2) fields directly — an amortized growable-array
/// push. Returns `nothing`.
pub(crate) fn build_list_push_fn() -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(2);
	let (list, x) = (w.param(0), w.param(1));
	let elems = w.local(types::valarray_ref());
	let len = w.local(ValType::I32);
	let cap = w.local(ValType::I32);
	let new = w.local(types::valarray_ref());

	// elems = list.elems; len = list.length; cap = |elems|.
	w.local_get(list)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(elems);
	w.local_get(list)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 2)
		.local_set(len);
	w.local_get(elems).array_len().local_set(cap);
	// if len >= cap: grow to max(cap*2, 4), copy, and swap in the new backing array.
	w.local_get(len).local_get(cap).i32_ge_u();
	w.if_(|w| {
		w.local_get(cap).i32_eqz();
		w.if_result(
			ValType::I32,
			|w| {
				w.i32(4);
			},
			|w| {
				w.local_get(cap).i32(1).i32_shl();
			},
		);
		w.array_new_default(va).local_set(new);
		w.copy_loop(va, new, None, elems, None, len);
		w.local_get(list)
			.ref_cast(types::T_LIST)
			.local_get(new)
			.struct_set(types::T_LIST, 1);
		w.local_get(new).local_set(elems);
	});
	// elems[len] = x; list.length = len + 1.
	w.local_get(elems).local_get(len).local_get(x).array_set(va);
	w.local_get(list)
		.ref_cast(types::T_LIST)
		.local_get(len)
		.i32(1)
		.i32_add()
		.struct_set(types::T_LIST, 2);
	// return nothing.
	w.ref_null(types::T_VALUE);
	w.finish()
}

/// Build `__list_pop(list) -> option`: remove and return the last element in
/// place, the O(1) dual of `__list_push`. Returns `some last` after decrementing
/// the logical length (field 2) by one — leaving the backing array's capacity
/// untouched but nulling the vacated slot so the popped value isn't kept alive —
/// or `none` when the list is empty.
pub(crate) fn build_list_pop_fn(opt: OptionLits) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(1);
	let list = w.param(0);
	let len = w.local(ValType::I32);
	let elems = w.local(types::valarray_ref());
	let x = w.local(types::value_ref());

	// len = list.length (logical count).
	list_len(&mut w, list);
	w.local_set(len);
	// if len == 0 -> none; else pop the last element.
	w.local_get(len).i32_eqz();
	w.if_result(
		types::value_ref(),
		|w| {
			build_none(w, opt);
		},
		|w| {
			// elems = list.elems; x = elems[len - 1].
			w.local_get(list)
				.ref_cast(types::T_LIST)
				.struct_get(types::T_LIST, 1)
				.local_set(elems);
			w.local_get(elems)
				.local_get(len)
				.i32(1)
				.i32_sub()
				.array_get(va)
				.local_set(x);
			// elems[len - 1] = null (don't pin the popped value past its lifetime).
			w.local_get(elems)
				.local_get(len)
				.i32(1)
				.i32_sub()
				.ref_null(types::T_VALUE)
				.array_set(va);
			// list.length = len - 1.
			w.local_get(list)
				.ref_cast(types::T_LIST)
				.local_get(len)
				.i32(1)
				.i32_sub()
				.struct_set(types::T_LIST, 2);
			// return some x.
			start_some(w, opt);
			w.local_get(x);
			finish_some(w);
		},
	);
	w.finish()
}

/// Build `__list_build(n, f) -> list`: tabulate `[f 0, f 1, ..., f (n-1)]` in
/// one pass. `arity1` is the wasm func-type index for a 1-arg closure (env-first
/// `(value, value) -> value`), used to `call_indirect` through `f`.
pub(crate) fn build_list_build_fn(arity1: u32) -> Function {
	let mut w = Wat::new(2);
	let (n, f) = (w.param(0), w.param(1));
	let nlen = w.local(ValType::I32);
	let buf = w.local(types::valarray_ref());
	let i = w.local(ValType::I32);

	// nlen = (int) n; buf = new valarray(nlen).
	w.local_get(n).unbox_int().i32_wrap_i64().local_set(nlen);
	w.local_get(nlen)
		.array_new_default(types::T_VALARRAY)
		.local_set(buf);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(nlen).i32_ge_s().br_if("brk");
			// buf[i] = f(box i).
			w.local_get(buf).local_get(i);
			w.local_get(f).ref_cast(types::T_CLOSURE); // env
			w.local_get(i).i64_extend_i32_s().box_int(); // arg = box i (i31 when small)
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1); // fn_index
			w.call_indirect(arity1);
			w.array_set(types::T_VALARRAY);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	mk_list(&mut w, buf);
	w.finish()
}

/// Build `__list_collect(n, f) -> list`: like `__list_build`, but `f` returns an
/// `option`; keep each `some`'s payload in order (detected by a non-empty variant
/// payload), then trim the over-allocated buffer to the kept count.
pub(crate) fn build_list_collect_fn(arity1: u32) -> Function {
	let mut w = Wat::new(2);
	let (n, f) = (w.param(0), w.param(1));
	let nlen = w.local(ValType::I32);
	let buf = w.local(types::valarray_ref());
	let i = w.local(ValType::I32);
	let write = w.local(ValType::I32); // kept count / write cursor
	let r = w.local(types::value_ref()); // f's result (an option variant)
	let out = w.local(types::valarray_ref());

	w.local_get(n).unbox_int().i32_wrap_i64().local_set(nlen);
	w.local_get(nlen)
		.array_new_default(types::T_VALARRAY)
		.local_set(buf);
	w.i32(0).local_set(i);
	w.i32(0).local_set(write);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(nlen).i32_ge_s().br_if("brk");
			// r = f(box i).
			w.local_get(f).ref_cast(types::T_CLOSURE);
			w.local_get(i).i64_extend_i32_s().box_int();
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1);
			w.call_indirect(arity1);
			w.local_set(r);
			// if r's arity is non-zero (some): buf[write] = payload[0]; write += 1.
			w.local_get(r)
				.ref_cast(types::T_VARIANT)
				.struct_get(types::T_VARIANT, 3);
			w.if_(|w| {
				w.local_get(buf).local_get(write);
				// `some`'s single payload element is the inline slot `p0` (field 4).
				w.local_get(r)
					.ref_cast(types::T_VARIANT)
					.struct_get(types::T_VARIANT, 4);
				w.array_set(types::T_VALARRAY);
				w.local_get(write).i32(1).i32_add().local_set(write);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	// out = new valarray(write); out[0..write] = buf[0..write].
	w.local_get(write)
		.array_new_default(types::T_VALARRAY)
		.local_set(out);
	w.copy_loop(types::T_VALARRAY, out, None, buf, None, write);
	mk_list(&mut w, out);
	w.finish()
}

/// Build `__arrconcat(a, b) -> valarray`: a fresh array holding `a` then `b`.
pub(crate) fn build_arrconcat_fn() -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(2);
	let (a, b) = (w.param(0), w.param(1));
	let la = w.local(ValType::I32);
	let lb = w.local(ValType::I32);
	let dst = w.local(types::valarray_ref());

	w.local_get(a).array_len().local_set(la);
	w.local_get(b).array_len().local_set(lb);
	// dst = new valarray(la + lb).
	w.local_get(la)
		.local_get(lb)
		.i32_add()
		.array_new_default(va)
		.local_set(dst);
	// dst[0..la] = a; dst[la..la+lb] = b — manual loops (see `Wat::copy_loop`).
	w.copy_loop(va, dst, None, a, None, la);
	w.copy_loop(va, dst, Some(la), b, None, lb);
	w.local_get(dst);
	w.finish()
}

/// Build `__run_defers(defers) -> nothing`: run a function's scheduled `defer`
/// cleanups LIFO at exit. `defers` is a `$list` of zero-arg cleanup closures the
/// emitter keeps in last-pushed-first order (each `defer` prepends), so walking
/// the backing `$valarray` front to back already runs them LIFO. Each thunk is a
/// `fun { … }`, which the module gives a phantom unit param (wasm arity 1), so
/// it's called with env + a dummy `nothing` arg; its result is discarded.
/// `thunk_ty` is that `(env, unit) -> value` `call_indirect` type.
pub(crate) fn build_run_defers_fn(thunk_ty: u32) -> Function {
	let mut w = Wat::new(1);
	let defers = w.param(0);
	let arr = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let c = w.local(types::value_ref());

	// arr = defers.elems; n = defers.length (logical count).
	w.local_get(defers)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(arr);
	list_len(&mut w, defers);
	w.local_set(n);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			// c = arr[i]; call c(()) with env = c and the phantom unit arg; discard.
			w.local_get(arr)
				.local_get(i)
				.array_get(types::T_VALARRAY)
				.local_set(c);
			w.local_get(c).ref_cast(types::T_CLOSURE); // env
			w.ref_null(types::T_VALUE); // phantom unit arg
			w.local_get(c)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1);
			w.call_indirect(thunk_ty);
			w.drop();
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	// Return `nothing`.
	w.ref_null(types::T_VALUE);
	w.finish()
}
