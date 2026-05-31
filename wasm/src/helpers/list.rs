// List helpers: `...rest` tail, tabulating builders, and value-array concat
// (`__list_tail`, `__list_build`, `__list_collect`, `__arrconcat`).

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::Wat;
use crate::types;

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

	// src = list.elems; len = |src|.
	w.local_get(list)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(src);
	w.local_get(src).array_len().local_set(len);
	// n = (int) narg.
	w.local_get(narg)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(n);
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
	w.i32(types::TAG_LIST)
		.local_get(dst)
		.struct_new(types::T_LIST);
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
	w.local_get(n)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(nlen);
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
			w.i32(types::TAG_INT)
				.local_get(i)
				.i64_extend_i32_s()
				.struct_new(types::T_INT); // arg = box i
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1); // fn_index
			w.call_indirect(arity1);
			w.array_set(types::T_VALARRAY);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.i32(types::TAG_LIST)
		.local_get(buf)
		.struct_new(types::T_LIST);
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

	w.local_get(n)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(nlen);
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
			w.i32(types::TAG_INT)
				.local_get(i)
				.i64_extend_i32_s()
				.struct_new(types::T_INT);
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1);
			w.call_indirect(arity1);
			w.local_set(r);
			// if r's payload is non-empty (some): buf[write] = payload[0]; write += 1.
			w.local_get(r)
				.ref_cast(types::T_VARIANT)
				.struct_get(types::T_VARIANT, 3)
				.array_len();
			w.if_(|w| {
				w.local_get(buf).local_get(write);
				w.local_get(r)
					.ref_cast(types::T_VARIANT)
					.struct_get(types::T_VARIANT, 3);
				w.i32(0).array_get(types::T_VALARRAY);
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
	w.i32(types::TAG_LIST)
		.local_get(out)
		.struct_new(types::T_LIST);
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

	// arr = defers.elems; n = len(arr).
	w.local_get(defers)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(arr);
	w.local_get(arr).array_len().local_set(n);
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
			w.i32(types::TAG_NOTHING).struct_new(types::T_VALUE); // phantom unit arg
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
	w.i32(types::TAG_NOTHING).struct_new(types::T_VALUE);
	w.finish()
}
