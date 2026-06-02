// Bytes helpers: tabulating builder + byte-array concat
// (`__bytes_build`, `__bytesconcat`).

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::Wat;
use crate::types;

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
