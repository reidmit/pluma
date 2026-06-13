// Bytes helpers: tabulating builder + byte-array concat
// (`__bytes_build`, `__bytesconcat`).

use crate::helpers::wat::Wat;
use crate::types;
use wasm_encoder::{Function, ValType};

/// Build `__bytes_build(n, f) -> bytes`: tabulate an `n`-byte sequence, calling
/// `f` per index and storing its int result (truncated to 8 bits by the packed
/// `$bytes` array). `arity1` is the 1-arg closure func-type index.
pub(crate) fn build_bytes_build_fn(arity1: u32) -> Function {
	let mut w = Wat::new(2);
	let (n, f) = (w.param(0), w.param(1));
	let nlen = w.local(ValType::I32);
	let buf = w.local(types::bytes_ref());
	let i = w.local(ValType::I32);

	w.local_get(n).unbox_int().i32_wrap_i64().local_set(nlen);
	w.local_get(nlen)
		.array_new_default(types::T_BYTES)
		.local_set(buf);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(nlen).i32_ge_s().br_if("brk");
			// buf[i] = (i32) f(box i).
			w.local_get(buf).local_get(i);
			w.local_get(f).ref_cast(types::T_CLOSURE); // env
			w.local_get(i).i64_extend_i32_s().box_int(); // arg = box i (i31 when small)
			w.local_get(f)
				.ref_cast(types::T_CLOSURE)
				.struct_get(types::T_CLOSURE, 1); // fn_index
			w.call_indirect(arity1);
			w.unbox_int().i32_wrap_i64(); // unbox result to i32 (array.set packs to i8)
			w.array_set(types::T_BYTES);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.i32(types::TAG_BYTES)
		.local_get(buf)
		.struct_new(types::T_STR);
	w.finish()
}

/// Build `__join(parts, sep) -> bytes`: glue a `$list` of strings/bytes into one
/// byte array with `sep` between adjacent parts, in a single pass. Sums every
/// part's (and separator's) length, allocates the result once, and blits each
/// piece into place — O(total) copy and exactly one allocation, versus the
/// binary-tree `concat` join's O(total·log k) copy and O(k) intermediate
/// allocations. `$str` and `$bytes` share the `$str` struct, so this serves both
/// `string.join` and `bytes.join`; the caller stamps the result tag.
pub(crate) fn build_join_fn() -> Function {
	let bv = types::T_BYTES;
	let mut w = Wat::new(2);
	let (list, sep) = (w.param(0), w.param(1));
	let elems = w.local(types::valarray_ref());
	let n = w.local(ValType::I32); // part count (list's logical length)
	let sepb = w.local(types::bytes_ref()); // sep's backing `$bytes`
	let seplen = w.local(ValType::I32);
	let total = w.local(ValType::I32);
	let dst = w.local(types::bytes_ref());
	let off = w.local(ValType::I32); // running write offset into `dst`
	let part = w.local(types::bytes_ref()); // current part's backing `$bytes`
	let plen = w.local(ValType::I32);
	let i = w.local(ValType::I32);

	// elems = list.elems; n = list.length (logical count, not capacity).
	w.local_get(list)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(elems);
	w.local_get(list)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 2)
		.local_set(n);
	// Fast paths that avoid allocating/copying a result: an empty list joins to an
	// empty byte array; a singleton joins to its sole part's bytes verbatim (no
	// separator applies). The latter is the `string.replace`-with-absent-pattern
	// path (`split` yields one piece), which must not copy.
	w.local_get(n).i32_eqz().if_(|w| {
		w.i32(0).array_new_default(bv).ret();
	});
	w.local_get(n).i32(1).i32_eq().if_(|w| {
		w.local_get(elems)
			.i32(0)
			.array_get(types::T_VALARRAY)
			.ref_cast(types::T_STR)
			.struct_get(types::T_STR, 1)
			.ret();
	});

	// sepb = sep.bytes; seplen = array.len(sepb).
	w.local_get(sep)
		.ref_cast(types::T_STR)
		.struct_get(types::T_STR, 1)
		.local_set(sepb);
	w.local_get(sepb).array_len().local_set(seplen);

	// Pass 1: total = Σ len(parts[i]) + max(0, n - 1) * seplen.
	w.i32(0).local_set(total);
	w.i32(0).local_set(i);
	w.block("sum_brk", |w| {
		w.loop_("sum_lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("sum_brk");
			w.local_get(total);
			w.local_get(elems)
				.local_get(i)
				.array_get(types::T_VALARRAY)
				.ref_cast(types::T_STR)
				.struct_get(types::T_STR, 1)
				.array_len();
			w.i32_add().local_set(total);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("sum_lp");
		});
	});
	w.local_get(n).i32(1).i32_gt_s().if_(|w| {
		w.local_get(total)
			.local_get(n)
			.i32(1)
			.i32_sub()
			.local_get(seplen)
			.i32_mul()
			.i32_add()
			.local_set(total);
	});

	// dst = new bytes(total).
	w.local_get(total).array_new_default(bv).local_set(dst);

	// Pass 2: blit each part into `dst`, with `sep` before parts 1..n-1.
	w.i32(0).local_set(off);
	w.i32(0).local_set(i);
	w.block("cp_brk", |w| {
		w.loop_("cp_lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("cp_brk");
			// Separator before every part but the first.
			w.local_get(i).i32(0).i32_gt_s().if_(|w| {
				w.copy_loop_bytes(bv, dst, Some(off), sepb, None, seplen);
				w.local_get(off).local_get(seplen).i32_add().local_set(off);
			});
			// part = parts[i].bytes; blit it; advance the offset.
			w.local_get(elems)
				.local_get(i)
				.array_get(types::T_VALARRAY)
				.ref_cast(types::T_STR)
				.struct_get(types::T_STR, 1)
				.local_set(part);
			w.local_get(part).array_len().local_set(plen);
			w.copy_loop_bytes(bv, dst, Some(off), part, None, plen);
			w.local_get(off).local_get(plen).i32_add().local_set(off);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("cp_lp");
		});
	});

	w.local_get(dst);
	w.finish()
}

/// Build `__bytesconcat(a, b) -> bytes`: a fresh byte array holding `a` then `b`.
pub(crate) fn build_bytesconcat_fn() -> Function {
	let bv = types::T_BYTES;
	let mut w = Wat::new(2);
	let (a, b) = (w.param(0), w.param(1));
	let la = w.local(ValType::I32);
	let lb = w.local(ValType::I32);
	let dst = w.local(types::bytes_ref());

	w.local_get(a).array_len().local_set(la);
	w.local_get(b).array_len().local_set(lb);
	w.local_get(la)
		.local_get(lb)
		.i32_add()
		.array_new_default(bv)
		.local_set(dst);
	// `dst[0..la] = a`, then `dst[la..la+lb] = b`, via explicit copy loops rather
	// than `array.copy` (a per-element libcall ~19x slower under wasmtime), since
	// `++`/join/interp fold through this helper hard — a tree of many small concats
	// was the string benchmark's bottleneck.
	w.copy_loop_bytes(bv, dst, None, a, None, la);
	w.copy_loop_bytes(bv, dst, Some(la), b, None, lb);
	w.local_get(dst);
	w.finish()
}
