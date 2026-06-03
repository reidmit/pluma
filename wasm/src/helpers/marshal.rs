// The wasm↔host marshalling primitives over the exported scratch linear memory.
//
// Phase-1 host imports never touch a GC `$value` field (a stock engine like V8 can't
// reflect one). Instead, byte payloads cross the boundary through a small exported
// linear memory: wasm encodes outgoing bytes into it and passes `(ptr, len)`; the
// host reads/writes that `ArrayBuffer` slice directly. These three helpers are the
// only things that bridge GC `$bytes` ⇄ linear memory:
//
//   * `__alloc(n)`        — reserve `n` scratch bytes (bump cursor + grow).
//   * `__store_bytes(b,p)`— copy a GC `$bytes` into scratch  (wasm → host).
//   * `__load_bytes(p,n)` — copy scratch bytes into a GC `$bytes`  (host → wasm).
//
// The bump cursor is a module global (`Runtime.bump`); a host-call emit site resets
// it to 0 before encoding that call's args, then bumps once per payload. Host calls
// are synchronous, so a payload only needs to outlive the single call it feeds.

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::Wat;
use crate::types;

/// `__alloc(n) -> ptr` — return the current bump cursor, advance it by `n`, and grow
/// the linear memory (one page at a time) until it holds the new top. The exported
/// memory starts at one page; reads of large payloads grow it here.
pub(crate) fn build_alloc_fn(bump: u32) -> Function {
	let mut w = Wat::new(1);
	let n = w.param(0);
	let p = w.local(ValType::I32);
	// p = bump; bump = p + n.
	w.global_get(bump).local_set(p);
	w.local_get(p).local_get(n).i32_add().global_set(bump);
	// Grow until `memory.size * 64KiB >= bump`.
	w.block("done", |w| {
		w.loop_("grow", |w| {
			// bump <= memory.size << 16 ?  → enough room, done.
			w.global_get(bump);
			w.memory_size().i32(16).i32_shl();
			w.i32_le_u().br_if("done");
			// else grow one page and retry.
			w.i32(1).memory_grow().drop();
			w.br("grow");
		});
	});
	w.local_get(p).ret();
	w.finish()
}

/// `__store_bytes(b, ptr) -> ()` — copy every byte of the GC `$bytes` array `b` into
/// the scratch memory starting at `ptr` (the wasm→host byte-payload primitive). The
/// caller is responsible for having `__alloc`'d `array.len(b)` bytes at `ptr`.
pub(crate) fn build_store_bytes_fn() -> Function {
	let bv = types::T_BYTES;
	let mut w = Wat::new(2);
	let b = w.param(0);
	let ptr = w.param(1);
	let i = w.local(ValType::I32);
	let len = w.local(ValType::I32);
	w.local_get(b).array_len().local_set(len);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(len).i32_ge_s().br_if("brk");
			// mem[ptr + i] = b[i] (unsigned byte).
			w.local_get(ptr).local_get(i).i32_add();
			w.local_get(b).local_get(i).array_get_u(bv);
			w.i32_store8();
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.finish()
}

/// `__send_bytes(b) -> len` — reset the bump cursor, copy the GC `$bytes` array `b`
/// into scratch at offset 0, and return its length. The single-payload convenience
/// the writer emit sites + the `print`-as-value wrapper share: after this returns,
/// the payload is at `(ptr=0, len)`, ready for a `(i32,i32) -> ()` writer import.
pub(crate) fn build_send_bytes_fn(bump: u32, alloc: u32, store: u32) -> Function {
	let mut w = Wat::new(1);
	let b = w.param(0);
	let len = w.local(ValType::I32);
	let ptr = w.local(ValType::I32);
	// Reset the cursor so the payload lands at offset 0.
	w.i32(0).global_set(bump);
	// len = array.len(b); ptr = __alloc(len) (= 0, but grows the memory if needed).
	w.local_get(b).array_len().local_set(len);
	w.local_get(len).call(alloc).local_set(ptr);
	w.local_get(b).local_get(ptr).call(store);
	w.local_get(len).ret();
	w.finish()
}

/// `__entry_error(value) -> i32 len` — probe `_entry`'s return for a program-level
/// failure (a `result.err e`) without the host reflecting the GC value: it shuttles
/// the opaque ref back in here. Detects an error structurally — a variant
/// whose display name's last `.`-segment is `err` with exactly one payload — then
/// renders `e` via `__tostring` into scratch and returns its length, or `-1` if the
/// return is not such an error (an `ok`, a plain value, `nothing`).
pub(crate) fn build_entry_error_fn(tostring: u32, send: u32) -> Function {
	let bv = types::T_BYTES;
	let mut w = Wat::new(1);
	let v = w.param(0);
	let nb = w.local(types::bytes_ref());
	let n = w.local(ValType::I32);
	let pl = w.local(types::valarray_ref());

	// Not a variant → not an error.
	w.local_get(v).value_tag().i32(types::TAG_VARIANT).i32_ne();
	w.if_(|w| {
		w.i32(-1).ret();
	});
	// name = the variant's display-name `$str` bytes (field 2).
	w.local_get(v)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 2)
		.ref_cast(types::T_STR)
		.struct_get(types::T_STR, 1)
		.local_set(nb);
	w.local_get(nb).array_len().local_set(n);
	// `rsplit('.').next() == "err"`: ≥3 bytes, ending "err", and either exactly "err"
	// or preceded by '.' (so "footerr" is rejected but "result.err" matches).
	let byte_at = |w: &mut Wat, back: i32| {
		w.local_get(nb)
			.local_get(n)
			.i32(back)
			.i32_sub()
			.array_get_u(bv);
	};
	w.local_get(n).i32(3).i32_lt_s();
	w.if_(|w| {
		w.i32(-1).ret();
	});
	byte_at(&mut w, 3);
	w.i32(0x65).i32_ne(); // 'e'
	w.if_(|w| {
		w.i32(-1).ret();
	});
	byte_at(&mut w, 2);
	w.i32(0x72).i32_ne(); // 'r'
	w.if_(|w| {
		w.i32(-1).ret();
	});
	byte_at(&mut w, 1);
	w.i32(0x72).i32_ne(); // 'r'
	w.if_(|w| {
		w.i32(-1).ret();
	});
	w.local_get(n).i32(3).i32_gt_s();
	w.if_(|w| {
		byte_at(w, 4);
		w.i32(0x2e).i32_ne(); // '.'
		w.if_(|w| {
			w.i32(-1).ret();
		});
	});
	// payload (field 3) must hold exactly one element.
	w.local_get(v)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 3)
		.local_set(pl);
	w.local_get(pl).array_len().i32(1).i32_ne();
	w.if_(|w| {
		w.i32(-1).ret();
	});
	// Render payload[0] via __tostring, copy into scratch, return its length.
	w.local_get(pl).i32(0).array_get(types::T_VALARRAY);
	w.call(tostring)
		.ref_cast(types::T_STR)
		.struct_get(types::T_STR, 1);
	w.call(send).ret();
	w.finish()
}

/// `__read_names(ptr, len) -> $list` — split a NUL-terminated name blob in scratch
/// (the `io.read-dir` host return: each entry name followed by a `\0`) into a `$list`
/// of `$str`. Counts the NULs for the element count, then materializes one `$str` per
/// segment via `__load_bytes`. An empty blob (`len == 0`) yields the empty list.
pub(crate) fn build_read_names_fn(load_bytes: u32) -> Function {
	let mut w = Wat::new(2);
	let ptr = w.param(0);
	let len = w.param(1);
	let count = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let arr = w.local(types::valarray_ref());
	let idx = w.local(ValType::I32);
	let seg_start = w.local(ValType::I32);

	// Pass 1: count = number of NUL terminators.
	w.i32(0).local_set(count);
	w.i32(0).local_set(i);
	w.block("c_brk", |w| {
		w.loop_("c_lp", |w| {
			w.local_get(i).local_get(len).i32_ge_s().br_if("c_brk");
			w.local_get(ptr)
				.local_get(i)
				.i32_add()
				.i32_load8_u()
				.i32_eqz();
			w.if_(|w| {
				w.local_get(count).i32(1).i32_add().local_set(count);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("c_lp");
		});
	});
	// arr = $valarray[count] (null slots, filled below).
	w.local_get(count)
		.array_new_default(types::T_VALARRAY)
		.local_set(arr);
	// Pass 2: one $str per NUL-terminated segment.
	w.i32(0).local_set(i);
	w.i32(0).local_set(idx);
	w.i32(0).local_set(seg_start);
	w.block("s_brk", |w| {
		w.loop_("s_lp", |w| {
			w.local_get(i).local_get(len).i32_ge_s().br_if("s_brk");
			w.local_get(ptr)
				.local_get(i)
				.i32_add()
				.i32_load8_u()
				.i32_eqz();
			w.if_(|w| {
				// arr[idx] = $str(__load_bytes(ptr + seg_start, i - seg_start))
				w.local_get(arr).local_get(idx);
				w.i32(types::TAG_STR);
				w.local_get(ptr).local_get(seg_start).i32_add();
				w.local_get(i).local_get(seg_start).i32_sub();
				w.call(load_bytes);
				w.struct_new(types::T_STR);
				w.array_set(types::T_VALARRAY);
				w.local_get(idx).i32(1).i32_add().local_set(idx);
				w.local_get(i).i32(1).i32_add().local_set(seg_start);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("s_lp");
		});
	});
	// $list { TAG_LIST, arr, count }.
	w.i32(types::TAG_LIST)
		.local_get(arr)
		.local_get(count)
		.struct_new(types::T_LIST)
		.ret();
	w.finish()
}

/// `__load_bytes(ptr, len) -> $bytes` — copy `len` bytes of scratch memory starting
/// at `ptr` into a fresh GC `$bytes` array (the host→wasm byte-payload primitive).
pub(crate) fn build_load_bytes_fn() -> Function {
	let bv = types::T_BYTES;
	let mut w = Wat::new(2);
	let ptr = w.param(0);
	let len = w.param(1);
	let out = w.local(types::bytes_ref());
	let i = w.local(ValType::I32);
	w.local_get(len).array_new_default(bv).local_set(out);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(len).i32_ge_s().br_if("brk");
			// out[i] = mem[ptr + i].
			w.local_get(out).local_get(i);
			w.local_get(ptr).local_get(i).i32_add().i32_load8_u();
			w.array_set(bv);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(out).ret();
	w.finish()
}
