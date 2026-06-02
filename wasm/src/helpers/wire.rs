// `wire` codec helpers: the FNV-1a mixers and the recursive schema fingerprint
// (`__wire_mix_len`, `__wire_mix_str`, `__wire_fp`), plus the native encode/decode
// over the `$value` GC layout.

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::{Local, Wat};
use crate::runtime::{WIRE_FNV_PRIME, WireGlobals, WireResultLits, WireTags};
use crate::types;

/// Build `__wire_mix_len(i64 h, i64 n) -> i64`: fold `mix_byte` over `n`'s 8
/// little-endian bytes (mirrors `vm::wire::mix_len`, where lengths are `u64` LE).
pub(crate) fn build_wire_mix_len_fn() -> Function {
	let mut w = Wat::new(2);
	let (h, n) = (w.param(0), w.param(1));
	let i = w.local(ValType::I32);

	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).i32(8).i32_ge_u().br_if("brk");
			// h = (h ^ ((n >> (i*8)) & 0xff)) * PRIME.
			w.local_get(h);
			w.local_get(n)
				.local_get(i)
				.i32(8)
				.i32_mul()
				.i64_extend_i32_u()
				.i64_shr_u();
			w.i64(0xff)
				.i64_and()
				.i64_xor()
				.i64(WIRE_FNV_PRIME)
				.i64_mul()
				.local_set(h);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(h);
	w.finish()
}

/// Build `__wire_mix_str(i64 h, ref $value str) -> i64`: mix the string's byte
/// length (via `mix_len`) then each of its bytes (mirrors `vm::wire::mix_str`).
pub(crate) fn build_wire_mix_str_fn(mix_len: u32) -> Function {
	let mut w = Wat::new(2);
	let (h, s) = (w.param(0), w.param(1));
	let bytes = w.local(types::bytes_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);

	// bytes = (cast $str s).field1.
	w.local_get(s)
		.ref_cast(types::T_STR)
		.struct_get(types::T_STR, 1)
		.local_set(bytes);
	// n = array.len bytes; h = mix_len(h, n).
	w.local_get(bytes).array_len().local_set(n);
	w.local_get(h)
		.local_get(n)
		.i64_extend_i32_u()
		.call(mix_len)
		.local_set(h);
	// for i in 0..n: h = (h ^ byte) * PRIME.
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
			w.local_get(h);
			w.local_get(bytes)
				.local_get(i)
				.array_get_u(types::T_BYTES)
				.i64_extend_i32_u();
			w.i64_xor().i64(WIRE_FNV_PRIME).i64_mul().local_set(h);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(h);
	w.finish()
}

/// Build `__wire_fp(i64 h, ref $value schema) -> i64`: the recursive structural
/// fingerprint over a `wire-schema` value tree (mirrors `vm::wire::mix_schema`).
/// Dispatches on the schema node's `vtag`; each arm leads with a distinct kind
/// byte (1..13) so structurally-different schemas can't alias. `self_idx` is this
/// function's own wasm index (for recursion).
pub(crate) fn build_wire_fp_fn(
	self_idx: u32,
	mix_str: u32,
	mix_len: u32,
	wt: WireTags,
) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(2);
	let (h, schema) = (w.param(0), w.param(1));
	let vtag = w.local(ValType::I32);
	let payload = w.local(types::valarray_ref());
	let elems = w.local(types::valarray_ref());
	let i = w.local(ValType::I32);
	let n = w.local(ValType::I32);
	let pe = w.local(types::valarray_ref());
	let fields = w.local(types::valarray_ref());
	let m = w.local(ValType::I32);
	let j = w.local(ValType::I32);

	// vtag = schema.variant_tag; payload = schema.payload.
	w.local_get(schema)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 1)
		.local_set(vtag);
	w.local_get(schema)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 3)
		.local_set(payload);

	// h = (h ^ kind) * PRIME, written back to h.
	let mix_byte = |w: &mut Wat, kind: i64| {
		w.local_get(h)
			.i64(kind)
			.i64_xor()
			.i64(WIRE_FNV_PRIME)
			.i64_mul()
			.local_set(h);
	};
	// Push payload[idx] (a `$value`).
	let payload_elem = |w: &mut Wat, idx: i32| {
		w.local_get(payload).i32(idx).array_get(va);
	};
	// dst = list-elems of payload[idx] (cast to `$list`, field 1).
	let elems_of = |w: &mut Wat, idx: i32, dst: Local| {
		w.local_get(payload).i32(idx).array_get(va);
		w.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(dst);
	};
	// Scalar arm: `if vtag == t { mix_byte(kind); return h }`.
	let scalar = |w: &mut Wat, t: u32, kind: i64| {
		w.local_get(vtag).i32(t as i32).i32_eq();
		w.if_(|w| {
			w.local_get(h)
				.i64(kind)
				.i64_xor()
				.i64(WIRE_FNV_PRIME)
				.i64_mul()
				.ret();
		});
	};
	scalar(&mut w, wt.s_int, 1);
	scalar(&mut w, wt.s_float, 2);
	scalar(&mut w, wt.s_bool, 3);
	scalar(&mut w, wt.s_string, 4);
	scalar(&mut w, wt.s_bytes, 5);
	scalar(&mut w, wt.s_duration, 6);
	scalar(&mut w, wt.s_nothing, 7);
	// s-list: wire_fp(mix_byte(h, 8), inner=payload[0]).
	w.local_get(vtag).i32(wt.s_list as i32).i32_eq();
	w.if_(|w| {
		mix_byte(w, 8);
		w.local_get(h);
		payload_elem(w, 0);
		w.call(self_idx).ret();
	});
	// s-dict: wire_fp(wire_fp(mix_byte(h, 12), k=payload[0]), v=payload[1]).
	w.local_get(vtag).i32(wt.s_dict as i32).i32_eq();
	w.if_(|w| {
		mix_byte(w, 12);
		w.local_get(h);
		payload_elem(w, 0);
		w.call(self_idx).local_set(h);
		w.local_get(h);
		payload_elem(w, 1);
		w.call(self_idx).ret();
	});
	// s-enum-ref: mix_str(mix_byte(h, 13), qualified=payload[0]).
	w.local_get(vtag).i32(wt.s_enum_ref as i32).i32_eq();
	w.if_(|w| {
		mix_byte(w, 13);
		w.local_get(h);
		payload_elem(w, 0);
		w.call(mix_str).ret();
	});
	// Fold `wire_fp` over the `$valarray` in `arr`, length `n`, using loop index
	// `idx`; accumulates into `h`.
	let fold_fp = |w: &mut Wat, arr: Local, idx: Local| {
		w.i32(0).local_set(idx);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(idx).local_get(n).i32_ge_u().br_if("brk");
				w.local_get(h);
				w.local_get(arr).local_get(idx).array_get(va);
				w.call(self_idx).local_set(h);
				w.local_get(idx).i32(1).i32_add().local_set(idx);
				w.br("lp");
			});
		});
	};
	// h = mix_len(h, (i64) local n).
	let mix_len_n = |w: &mut Wat| {
		w.local_get(h)
			.local_get(n)
			.i64_extend_i32_u()
			.call(mix_len)
			.local_set(h);
	};
	// s-tuple: mix_len(mix_byte(h, 9), elems.len()); fold wire_fp over elems.
	w.local_get(vtag).i32(wt.s_tuple as i32).i32_eq();
	w.if_(|w| {
		mix_byte(w, 9);
		elems_of(w, 0, elems);
		w.local_get(elems).array_len().local_set(n);
		mix_len_n(w);
		fold_fp(w, elems, i);
		w.local_get(h).ret();
	});
	// s-record: mix_len(mix_byte(h, 10), fields.len()); each field is a
	// `$tuple (name, schema)` — mix_str the name, recurse on the schema.
	w.local_get(vtag).i32(wt.s_record as i32).i32_eq();
	w.if_(|w| {
		mix_byte(w, 10);
		elems_of(w, 0, elems);
		w.local_get(elems).array_len().local_set(n);
		mix_len_n(w);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
				// pe = (cast $tuple elems[i]).field1.
				w.local_get(elems).local_get(i).array_get(va);
				w.ref_cast(types::T_TUPLE)
					.struct_get(types::T_TUPLE, 1)
					.local_set(pe);
				// h = mix_str(h, pe[0]).
				w.local_get(h);
				w.local_get(pe).i32(0).array_get(va);
				w.call(mix_str).local_set(h);
				// h = wire_fp(h, pe[1]).
				w.local_get(h);
				w.local_get(pe).i32(1).array_get(va);
				w.call(self_idx).local_set(h);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.local_get(h).ret();
	});
	// s-enum: mix_len(mix_str(mix_byte(h, 11), qualified), variants.len()); each
	// variant is a `$tuple (name, list-of-field-schemas)`.
	w.local_get(vtag).i32(wt.s_enum as i32).i32_eq();
	w.if_(|w| {
		mix_byte(w, 11);
		w.local_get(h);
		payload_elem(w, 0);
		w.call(mix_str).local_set(h);
		// elems = variants list (payload[1] is a `$list`).
		w.local_get(payload).i32(1).array_get(va);
		w.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(elems);
		w.local_get(elems).array_len().local_set(n);
		mix_len_n(w);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
				// pe = (cast $tuple variants[i]).field1  (name, field-list).
				w.local_get(elems).local_get(i).array_get(va);
				w.ref_cast(types::T_TUPLE)
					.struct_get(types::T_TUPLE, 1)
					.local_set(pe);
				// h = mix_str(h, pe[0])  (variant name).
				w.local_get(h);
				w.local_get(pe).i32(0).array_get(va);
				w.call(mix_str).local_set(h);
				// fields = (cast $list pe[1]).field1; m = len; h = mix_len(h, m).
				w.local_get(pe).i32(1).array_get(va);
				w.ref_cast(types::T_LIST)
					.struct_get(types::T_LIST, 1)
					.local_set(fields);
				w.local_get(fields).array_len().local_set(m);
				w.local_get(h)
					.local_get(m)
					.i64_extend_i32_u()
					.call(mix_len)
					.local_set(h);
				// for j in 0..m: h = wire_fp(h, fields[j]).
				w.i32(0).local_set(j);
				w.block("ibrk", |w| {
					w.loop_("ilp", |w| {
						w.local_get(j).local_get(m).i32_ge_u().br_if("ibrk");
						w.local_get(h);
						w.local_get(fields).local_get(j).array_get(va);
						w.call(self_idx).local_set(h);
						w.local_get(j).i32(1).i32_add().local_set(j);
						w.br("ilp");
					});
				});
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.local_get(h).ret();
	});
	// Fallthrough (malformed schema): return h unchanged.
	w.local_get(h);
	w.finish()
}

// ===========================================================================
// `wire` codec: encode / decode native over the `$value` GC layout.
//
// The codec interprets a `wire-schema` value tree (the same tree `__wire_fp`
// fingerprints) to drive a positional binary encode/decode, mirroring
// `vm::wire` byte-for-byte. State lives in module-level mutable globals
// (`WireGlobals`) rather than threading through the recursion: encode appends to
// a doubling byte buffer; decode reads a cursor and reports failure through an
// error code; both register inline enum definitions in a small registry so a
// recursive `s-enum-ref` resolves to its enclosing `s-enum`.
// ===========================================================================

/// Push a fresh `$str` for an interned data-segment literal `(off, len)`.
fn str_lit(w: &mut Wat, (off, len): (u32, u32)) {
	w.i32(types::TAG_STR);
	w.i32(off as i32);
	w.i32(len as i32);
	w.array_new_data(types::T_BYTES, 0);
	w.struct_new(types::T_STR);
}

/// Push the unit `nothing` value.
fn push_nothing(w: &mut Wat) {
	w.ref_null(types::T_VALUE);
}

/// Build `__wire_push(i32 b)`: append `b` to the encode buffer `g_buf`, growing
/// it (doubling) when full. `g_buf` is pre-initialized non-null at the call site,
/// so `array.len`/`array.set` never see null.
pub(crate) fn build_wire_push_fn(g: WireGlobals) -> Function {
	let bytes = types::T_BYTES;
	let mut w = Wat::new(1);
	let b = w.param(0);
	let new = w.local(types::bytes_ref());
	let src = w.local(types::bytes_ref());
	let len = w.local(ValType::I32);

	// if g_len >= array.len(g_buf): grow. (The buffer is reused across encode
	// calls, so this grow path runs only while the buffer is still smaller than
	// the payload — after warmup it's a single never-taken branch.)
	w.global_get(g.len).global_get(g.buf).array_len().i32_ge_u();
	w.if_(|w| {
		// new = array.new_default $bytes (cap * 2).
		w.global_get(g.buf)
			.array_len()
			.i32(1)
			.i32_shl()
			.array_new_default(bytes)
			.local_set(new);
		// new[0..g_len] = g_buf[0..g_len] (loop, not array.copy — see copy_loop_bytes).
		// g_buf is non-null here (pre-initialized at the call site), so cast away the
		// global's nullability for the non-null copy source.
		w.global_get(g.len).local_set(len);
		w.global_get(g.buf).ref_cast(bytes).local_set(src);
		w.copy_loop_bytes(bytes, new, None, src, None, len);
		w.local_get(new).global_set(g.buf);
	});
	// g_buf[g_len] = b.
	w.global_get(g.buf)
		.global_get(g.len)
		.local_get(b)
		.array_set(bytes);
	// g_len += 1.
	w.global_get(g.len).i32(1).i32_add().global_set(g.len);
	w.finish()
}

/// Build `__wire_uvarint(i64 v)`: write `v` as an LEB128 unsigned varint via
/// `__wire_push` (mirrors `vm::wire::write_uvarint`).
pub(crate) fn build_wire_uvarint_fn(push: u32) -> Function {
	let mut w = Wat::new(1);
	let v = w.param(0);
	let byte = w.local(ValType::I32);

	w.loop_("lp", |w| {
		// byte = v & 0x7f.
		w.local_get(v)
			.i64(0x7f)
			.i64_and()
			.i32_wrap_i64()
			.local_set(byte);
		// v >>= 7 (unsigned).
		w.local_get(v).i64(7).i64_shr_u().local_set(v);
		// if v == 0: push(byte); return.
		w.local_get(v).i64_eqz();
		w.if_(|w| {
			w.local_get(byte).call(push).ret();
		});
		// else push(byte | 0x80); continue.
		w.local_get(byte).i32(0x80).i32_or().call(push);
		w.br("lp");
	});
	w.finish()
}

/// Build `__wire_ctxput(value qualified, value variants) -> value`: register the
/// inline enum `(qualified, variants)` in the recursive-enum registry `g_ctx`
/// (append, growing by doubling), returning `variants` for convenience.
pub(crate) fn build_wire_ctxput_fn(g: WireGlobals) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(2);
	let (qual, vars) = (w.param(0), w.param(1));
	let new = w.local(types::valarray_ref());
	let src = w.local(types::valarray_ref_null());
	let len = w.local(ValType::I32);

	// if g_ctxlen >= array.len(g_ctx): grow.
	w.global_get(g.ctxlen)
		.global_get(g.ctx)
		.array_len()
		.i32_ge_u();
	w.if_(|w| {
		w.global_get(g.ctx)
			.array_len()
			.i32(1)
			.i32_shl()
			.array_new_default(va)
			.local_set(new);
		w.global_get(g.ctxlen).local_set(len);
		w.global_get(g.ctx).local_set(src);
		w.copy_loop(va, new, None, src, None, len);
		w.local_get(new).global_set(g.ctx);
	});
	// g_ctx[g_ctxlen] = tuple(qualified, variants).
	w.global_get(g.ctx).global_get(g.ctxlen);
	w.i32(types::TAG_TUPLE)
		.local_get(qual)
		.local_get(vars)
		.array_new_fixed(va, 2)
		.struct_new(types::T_TUPLE);
	w.array_set(va);
	// g_ctxlen += 1.
	w.global_get(g.ctxlen).i32(1).i32_add().global_set(g.ctxlen);
	// return variants.
	w.local_get(vars);
	w.finish()
}

/// Build `__wire_ctxget(value qualified) -> value`: linear-scan `g_ctx` for the
/// entry whose name `__eq` `qualified`, returning its variants `$list` (or null
/// if unregistered — the decoder treats null as a malformed back-reference).
pub(crate) fn build_wire_ctxget_fn(eq: u32, g: WireGlobals) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(1);
	let qual = w.param(0);
	let i = w.local(ValType::I32);
	let entry = w.local(types::valarray_ref());

	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).global_get(g.ctxlen).i32_ge_u().br_if("brk");
			// entry = (cast $tuple g_ctx[i]).elems.
			w.global_get(g.ctx).local_get(i).array_get(va);
			w.ref_cast(types::T_TUPLE)
				.struct_get(types::T_TUPLE, 1)
				.local_set(entry);
			// if __eq(entry[0], qualified): return entry[1].
			w.local_get(entry).i32(0).array_get(va);
			w.local_get(qual).call(eq);
			w.if_(|w| {
				w.local_get(entry).i32(1).array_get(va).ret();
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	// not found: null.
	w.ref_null(types::T_VALUE);
	w.finish()
}

/// Build `__wire_enc(value schema, value val)`: the recursive encoder. Dispatches
/// on the schema node's `vtag` (resolved via `WireTags`) and appends `val`'s
/// positional binary encoding to `g_buf` (mirrors `vm::wire::encode_in`).
/// `self_idx` is this function's own index (recursion); `enc_variant` encodes an
/// enum payload.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_wire_enc_fn(
	self_idx: u32,
	push: u32,
	uvarint: u32,
	ctxput: u32,
	ctxget: u32,
	enc_variant: u32,
	enc_dict: u32,
	wt: WireTags,
) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(2);
	let (schema, val) = (w.param(0), w.param(1));
	let vtag = w.local(ValType::I32);
	let payload = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let elems = w.local(types::valarray_ref());
	let schemas = w.local(types::valarray_ref());
	let bits = w.local(ValType::I64);
	let bytes = w.local(types::bytes_ref());

	// vtag = schema.variant_tag; payload = schema.payload.
	w.local_get(schema)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 1)
		.local_set(vtag);
	w.local_get(schema)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 3)
		.local_set(payload);

	let payload_elem = |w: &mut Wat, idx: i32| {
		w.local_get(payload).i32(idx).array_get(va);
	};
	// int / duration: uvarint(zigzag(unbox-i64 val)).
	let int_arm = |w: &mut Wat, t: u32| {
		w.local_get(vtag).i32(t as i32).i32_eq();
		w.if_(|w| {
			// (n << 1) ^ (n >> 63), recomputing `n` (cheap) rather than spilling.
			w.local_get(val).unbox_int().i64(1).i64_shl();
			w.local_get(val).unbox_int().i64(63).i64_shr_s();
			w.i64_xor().call(uvarint).ret();
		});
	};
	int_arm(&mut w, wt.s_int);
	int_arm(&mut w, wt.s_duration);
	// float: 8 little-endian bytes of the IEEE-754 bit pattern.
	w.local_get(vtag).i32(wt.s_float as i32).i32_eq();
	w.if_(|w| {
		w.local_get(val)
			.ref_cast(types::T_FLOAT)
			.struct_get(types::T_FLOAT, 1)
			.i64_reinterpret_f64()
			.local_set(bits);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).i32(8).i32_ge_u().br_if("brk");
				w.local_get(bits)
					.local_get(i)
					.i32(8)
					.i32_mul()
					.i64_extend_i32_u()
					.i64_shr_u();
				w.i64(0xff).i64_and().i32_wrap_i64().call(push);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.ret();
	});
	// bool: one byte.
	w.local_get(vtag).i32(wt.s_bool as i32).i32_eq();
	w.if_(|w| {
		w.local_get(val)
			.ref_cast(types::T_BOOL)
			.struct_get(types::T_BOOL, 1)
			.call(push)
			.ret();
	});
	// string / bytes: uvarint(len) then the raw bytes (both reuse `$str` shape).
	let bytes_arm = |w: &mut Wat, t: u32| {
		w.local_get(vtag).i32(t as i32).i32_eq();
		w.if_(|w| {
			w.local_get(val)
				.ref_cast(types::T_STR)
				.struct_get(types::T_STR, 1)
				.local_set(bytes);
			w.local_get(bytes).array_len().local_set(n);
			w.local_get(n).i64_extend_i32_u().call(uvarint);
			w.i32(0).local_set(i);
			w.block("brk", |w| {
				w.loop_("lp", |w| {
					w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
					w.local_get(bytes)
						.local_get(i)
						.array_get_u(types::T_BYTES)
						.call(push);
					w.local_get(i).i32(1).i32_add().local_set(i);
					w.br("lp");
				});
			});
			w.ret();
		});
	};
	bytes_arm(&mut w, wt.s_string);
	bytes_arm(&mut w, wt.s_bytes);
	// nothing: zero bytes.
	w.local_get(vtag).i32(wt.s_nothing as i32).i32_eq();
	w.if_(|w| {
		w.ret();
	});
	// list: uvarint(count) then each element under the inner schema (payload[0]).
	w.local_get(vtag).i32(wt.s_list as i32).i32_eq();
	w.if_(|w| {
		w.local_get(val)
			.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(elems);
		// the logical length (field 2), not array.len (capacity).
		w.local_get(val)
			.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 2)
			.local_set(n);
		w.local_get(n).i64_extend_i32_u().call(uvarint);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
				payload_elem(w, 0);
				w.local_get(elems).local_get(i).array_get(va);
				w.call(self_idx);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.ret();
	});
	// tuple: each field in order; schemas = list-elems of payload[0], values =
	// the `$tuple`'s own elems (arity matches, no count on the wire).
	w.local_get(vtag).i32(wt.s_tuple as i32).i32_eq();
	w.if_(|w| {
		payload_elem(w, 0);
		w.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(schemas);
		w.local_get(val)
			.ref_cast(types::T_TUPLE)
			.struct_get(types::T_TUPLE, 1)
			.local_set(elems);
		w.local_get(schemas).array_len().local_set(n);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
				w.local_get(schemas).local_get(i).array_get(va);
				w.local_get(elems).local_get(i).array_get(va);
				w.call(self_idx);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.ret();
	});
	// record: field schemas = list of `$tuple(name, schema)` in payload[0],
	// canonical (name-sorted) order; the `$record`'s values array is the same
	// order, so encode positionally (mirrors VM's per-name lookup).
	w.local_get(vtag).i32(wt.s_record as i32).i32_eq();
	w.if_(|w| {
		payload_elem(w, 0);
		w.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(schemas);
		w.local_get(val)
			.ref_cast(types::T_RECORD)
			.struct_get(types::T_RECORD, 2)
			.local_set(elems);
		w.local_get(schemas).array_len().local_set(n);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
				// schema = (cast $tuple schemas[i]).elems[1].
				w.local_get(schemas).local_get(i).array_get(va);
				w.ref_cast(types::T_TUPLE)
					.struct_get(types::T_TUPLE, 1)
					.i32(1)
					.array_get(va);
				w.local_get(elems).local_get(i).array_get(va);
				w.call(self_idx);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.ret();
	});
	// enum: register `(qualified, variants)` for any inner `s-enum-ref`, then
	// encode the variant tag + payload.
	w.local_get(vtag).i32(wt.s_enum as i32).i32_eq();
	w.if_(|w| {
		payload_elem(w, 0);
		payload_elem(w, 1);
		w.call(ctxput).drop();
		payload_elem(w, 1);
		w.local_get(val).call(enc_variant).ret();
	});
	// enum-ref: resolve the registered variants by name, then encode.
	w.local_get(vtag).i32(wt.s_enum_ref as i32).i32_eq();
	w.if_(|w| {
		payload_elem(w, 0);
		w.call(ctxget);
		w.local_get(val).call(enc_variant).ret();
	});
	// dict: canonical key-sorted encode (its own helper — needs scratch state for
	// the key bytes + sort).
	w.local_get(vtag).i32(wt.s_dict as i32).i32_eq();
	w.if_(|w| {
		w.local_get(schema).local_get(val).call(enc_dict).ret();
	});
	// Fallthrough (unreachable for well-typed values): emit nothing.
	w.finish()
}

/// Build `__wire_enc_variant(value variants, value val)`: write the variant's
/// declaration-index tag (a uvarint) then encode each payload field under its
/// schema (mirrors `vm::wire::encode_variant`). `variants` is the enum's variant
/// `$list` (`$tuple(name, field-schema-list)` per variant); the value's own
/// `vtag` is the wire tag and the index into `variants`.
pub(crate) fn build_wire_enc_variant_fn(enc: u32, uvarint: u32) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(2);
	let (variants, val) = (w.param(0), w.param(1));
	let varelems = w.local(types::valarray_ref());
	let vvtag = w.local(ValType::I32);
	let fschemas = w.local(types::valarray_ref());
	let pv = w.local(types::valarray_ref());
	let m = w.local(ValType::I32);
	let j = w.local(ValType::I32);

	// varelems = (cast $list variants).elems.
	w.local_get(variants)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(varelems);
	// vvtag = (cast $variant val).variant_tag; uvarint(vvtag).
	w.local_get(val)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 1)
		.local_set(vvtag);
	w.local_get(vvtag).i64_extend_i32_u().call(uvarint);
	// fschemas = (cast $list (cast $tuple varelems[vvtag]).elems[1]).elems.
	w.local_get(varelems).local_get(vvtag).array_get(va);
	w.ref_cast(types::T_TUPLE)
		.struct_get(types::T_TUPLE, 1)
		.i32(1)
		.array_get(va);
	w.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(fschemas);
	// pv = (cast $variant val).payload; m = len(fschemas).
	w.local_get(val)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 3)
		.local_set(pv);
	w.local_get(fschemas).array_len().local_set(m);
	// for j in 0..m: enc(fschemas[j], pv[j]).
	w.i32(0).local_set(j);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(j).local_get(m).i32_ge_u().br_if("brk");
			w.local_get(fschemas).local_get(j).array_get(va);
			w.local_get(pv).local_get(j).array_get(va);
			w.call(enc);
			w.local_get(j).i32(1).i32_add().local_set(j);
			w.br("lp");
		});
	});
	w.finish()
}

/// Build `__wire_rbyte() -> i32`: read one input byte, advancing `g_pos`. Once
/// `g_err` is set (or the cursor is at end) it's a no-op returning 0, so the
/// first error wins and over-reads don't trap.
pub(crate) fn build_wire_rbyte_fn(g: WireGlobals) -> Function {
	let mut w = Wat::new(0);
	let byte = w.local(ValType::I32);

	// already failed: preserve the first error, return 0.
	w.global_get(g.err);
	w.if_(|w| {
		w.i32(0).ret();
	});
	// out of bytes: unexpected-end.
	w.global_get(g.pos)
		.global_get(g.input)
		.array_len()
		.i32_ge_u();
	w.if_(|w| {
		w.i32(1).global_set(g.err);
		w.i32(0).ret();
	});
	// b = g_in[g_pos]; g_pos += 1.
	w.global_get(g.input)
		.global_get(g.pos)
		.array_get_u(types::T_BYTES)
		.local_set(byte);
	w.global_get(g.pos).i32(1).i32_add().global_set(g.pos);
	w.local_get(byte);
	w.finish()
}

/// Build `__wire_ruvarint() -> i64`: read an LEB128 unsigned varint (10-byte cap;
/// overlong/unterminated → `g_err = 5` malformed). Mirrors `vm::wire::read_uvarint`.
pub(crate) fn build_wire_ruvarint_fn(rbyte: u32, g: WireGlobals) -> Function {
	let mut w = Wat::new(0);
	let result = w.local(ValType::I64);
	let shift = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let byte = w.local(ValType::I32);

	w.i64(0).local_set(result);
	w.i32(0).local_set(shift);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			// 10 bytes consumed without terminator → malformed (handled after block).
			w.local_get(i).i32(10).i32_ge_u().br_if("brk");
			// byte = rbyte(); bail if that errored.
			w.call(rbyte).local_set(byte);
			w.global_get(g.err);
			w.if_(|w| {
				w.i64(0).ret();
			});
			// on the 10th byte (i==9) only the low bit is valid for a 64-bit int.
			w.local_get(i).i32(9).i32_eq();
			w.local_get(byte).i32(1).i32_gt_u();
			w.i32_and();
			w.if_(|w| {
				w.i32(5).global_set(g.err);
				w.i64(0).ret();
			});
			// result |= (byte & 0x7f) << shift.
			w.local_get(result);
			w.local_get(byte).i32(0x7f).i32_and().i64_extend_i32_u();
			w.local_get(shift).i64_extend_i32_u().i64_shl();
			w.i64_or().local_set(result);
			// high bit clear → done.
			w.local_get(byte).i32(0x80).i32_and().i32_eqz();
			w.if_(|w| {
				w.local_get(result).ret();
			});
			w.local_get(shift).i32(7).i32_add().local_set(shift);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	// loop exhausted: malformed.
	w.i32(5).global_set(g.err);
	w.i64(0);
	w.finish()
}

/// Build `__wire_disp(value qualified, value varname) -> value`: rebuild a
/// decoded variant's display name `"<bare-enum>.<variant>"` (bare = the qualified
/// name after its last `.`), so `to-string`/the host formatter render it like a
/// literally-constructed variant. Equality/pattern-match use the `vtag`, not this.
pub(crate) fn build_wire_disp_fn(bytesconcat: u32) -> Function {
	let bytes = types::T_BYTES;
	let mut w = Wat::new(2);
	let (qual, varname) = (w.param(0), w.param(1));
	let qb = w.local(types::bytes_ref());
	let n = w.local(ValType::I32);
	let last = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let start = w.local(ValType::I32);
	let barelen = w.local(ValType::I32);
	let bare = w.local(types::bytes_ref());

	// qb = qualified bytes; n = len.
	w.local_get(qual)
		.ref_cast(types::T_STR)
		.struct_get(types::T_STR, 1)
		.local_set(qb);
	w.local_get(qb).array_len().local_set(n);
	// last = index of the last '.' (46), or -1.
	w.i32(-1).local_set(last);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
			w.local_get(qb)
				.local_get(i)
				.array_get_u(bytes)
				.i32(46)
				.i32_eq();
			w.if_(|w| {
				w.local_get(i).local_set(last);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	// start = last + 1; barelen = n - start.
	w.local_get(last).i32(1).i32_add().local_set(start);
	w.local_get(n).local_get(start).i32_sub().local_set(barelen);
	// bare = qb[start..n] (loop, not array.copy — see copy_loop_bytes).
	w.local_get(barelen)
		.array_new_default(bytes)
		.local_set(bare);
	w.copy_loop_bytes(bytes, bare, None, qb, Some(start), barelen);
	// result = $str( (bare ++ ".") ++ varname-bytes ).
	w.i32(types::TAG_STR);
	w.local_get(bare)
		.i32(46)
		.array_new_fixed(bytes, 1)
		.call(bytesconcat);
	w.local_get(varname)
		.ref_cast(types::T_STR)
		.struct_get(types::T_STR, 1)
		.call(bytesconcat);
	w.struct_new(types::T_STR);
	w.finish()
}

/// Build `__wire_dec_variant(value qualified, value variants) -> value`: read the
/// variant tag (a uvarint), bounds-check it against `variants`, decode each
/// payload field, and build the `$variant` (mirrors `vm::wire::decode_variant`).
/// An out-of-range tag sets `g_err = 2` (invalid-tag, `g_errval` = tag).
pub(crate) fn build_wire_dec_variant_fn(
	ruvarint: u32,
	dec: u32,
	disp: u32,
	g: WireGlobals,
) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(2);
	let (qual, variants) = (w.param(0), w.param(1));
	let tag = w.local(ValType::I64);
	let varelems = w.local(types::valarray_ref());
	let m = w.local(ValType::I32);
	let idx = w.local(ValType::I32);
	let tup = w.local(types::valarray_ref());
	let name = w.local(types::value_ref());
	let fsl = w.local(types::valarray_ref());
	let k = w.local(ValType::I32);
	let j = w.local(ValType::I32);
	let payload = w.local(types::valarray_ref());
	let disp_l = w.local(types::value_ref());

	// tag = ruvarint(); bail on read failure.
	w.call(ruvarint).local_set(tag);
	w.global_get(g.err);
	w.if_(|w| {
		push_nothing(w);
		w.ret();
	});
	// varelems = (cast $list variants).elems; m = len.
	w.local_get(variants)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(varelems);
	w.local_get(varelems).array_len().local_set(m);
	// if tag < 0 || tag >= m: invalid-tag.
	w.local_get(tag).i64(0).i64_lt_s();
	w.local_get(tag).local_get(m).i64_extend_i32_u().i64_ge_s();
	w.i32_or();
	w.if_(|w| {
		w.i32(2).global_set(g.err);
		w.local_get(tag).global_set(g.errval);
		push_nothing(w);
		w.ret();
	});
	w.local_get(tag).i32_wrap_i64().local_set(idx);
	// tup = (cast $tuple varelems[idx]).elems; name = tup[0]; fsl = (cast $list
	// tup[1]).elems; k = len.
	w.local_get(varelems).local_get(idx).array_get(va);
	w.ref_cast(types::T_TUPLE)
		.struct_get(types::T_TUPLE, 1)
		.local_set(tup);
	w.local_get(tup).i32(0).array_get(va).local_set(name);
	w.local_get(tup).i32(1).array_get(va);
	w.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(fsl);
	w.local_get(fsl).array_len().local_set(k);
	// payload = $valarray(k); for j: payload[j] = dec(fsl[j]).
	w.local_get(k).array_new_default(va).local_set(payload);
	w.i32(0).local_set(j);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(j).local_get(k).i32_ge_u().br_if("brk");
			w.local_get(payload).local_get(j);
			w.local_get(fsl).local_get(j).array_get(va);
			w.call(dec).array_set(va);
			w.local_get(j).i32(1).i32_add().local_set(j);
			w.br("lp");
		});
	});
	// disp_l = disp(qualified, name); build $variant{tag, idx, disp_l, payload}.
	w.local_get(qual)
		.local_get(name)
		.call(disp)
		.local_set(disp_l);
	w.i32(types::TAG_VARIANT)
		.local_get(idx)
		.local_get(disp_l)
		.local_get(payload)
		.struct_new(types::T_VARIANT);
	w.finish()
}

/// Build `__wire_dec(value schema) -> value`: the recursive decoder. Dispatches on
/// the schema's `vtag`; reads/recursion set `g_err` on failure and the partial
/// value is discarded by `__wire_result`. Mirrors `vm::wire::decode_in`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_wire_dec_fn(
	self_idx: u32,
	ruvarint: u32,
	rbyte: u32,
	ctxput: u32,
	ctxget: u32,
	dec_variant: u32,
	dict_insert: u32,
	g: WireGlobals,
	wt: WireTags,
) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(1);
	let schema = w.param(0);
	let vtag = w.local(ValType::I32);
	let payload = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let schemas = w.local(types::valarray_ref());
	let out = w.local(types::valarray_ref());
	let names = w.local(types::valarray_ref());
	let values = w.local(types::valarray_ref());
	let fields = w.local(types::valarray_ref());
	let ft = w.local(types::valarray_ref());
	let inner = w.local(types::value_ref());
	let u = w.local(ValType::I64);
	let bits = w.local(ValType::I64);
	let len = w.local(ValType::I32);
	let bytes = w.local(types::bytes_ref());
	let vars = w.local(types::value_ref());

	w.local_get(schema)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 1)
		.local_set(vtag);
	w.local_get(schema)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 3)
		.local_set(payload);

	let payload_elem = |w: &mut Wat, idx: i32| {
		w.local_get(payload).i32(idx).array_get(va);
	};
	// if g_err: return nothing immediately.
	let bail = |w: &mut Wat| {
		w.global_get(g.err);
		w.if_(|w| {
			push_nothing(w);
			w.ret();
		});
	};
	// int / duration: box(unzigzag(uvarint)).
	let int_arm = |w: &mut Wat, t: u32, tag: i32| {
		w.local_get(vtag).i32(t as i32).i32_eq();
		w.if_(|w| {
			w.call(ruvarint).local_set(u);
			bail(w);
			w.i32(tag);
			// (u >>u 1) ^ (0 - (u & 1)).
			w.local_get(u).i64(1).i64_shr_u();
			w.i64(0).local_get(u).i64(1).i64_and().i64_sub();
			w.i64_xor().struct_new(types::T_INT).ret();
		});
	};
	int_arm(&mut w, wt.s_int, types::TAG_INT);
	int_arm(&mut w, wt.s_duration, types::TAG_DURATION);
	// float: 8 LE bytes → i64 → f64.
	w.local_get(vtag).i32(wt.s_float as i32).i32_eq();
	w.if_(|w| {
		w.i64(0).local_set(bits);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).i32(8).i32_ge_u().br_if("brk");
				w.local_get(bits);
				w.call(rbyte).i64_extend_i32_u().i64(0xff).i64_and();
				w.local_get(i).i32(8).i32_mul().i64_extend_i32_u().i64_shl();
				w.i64_or().local_set(bits);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		bail(w);
		w.i32(types::TAG_FLOAT)
			.local_get(bits)
			.f64_reinterpret_i64()
			.struct_new(types::T_FLOAT)
			.ret();
	});
	// bool: one byte != 0.
	w.local_get(vtag).i32(wt.s_bool as i32).i32_eq();
	w.if_(|w| {
		w.i32(types::TAG_BOOL)
			.call(rbyte)
			.i32(0)
			.i32_ne()
			.struct_new(types::T_BOOL)
			.ret();
	});
	// string / bytes: uvarint length, then that many bytes. NOTE: strings are
	// taken verbatim — the VM validates UTF-8 (the `invalid-utf8` error,
	// `g_err = 3`) but we don't yet, so a non-UTF-8 wire string decodes to a
	// malformed string here rather than erroring. Unexercised by the fixtures.
	let bytes_arm = |w: &mut Wat, t: u32, tag: i32| {
		w.local_get(vtag).i32(t as i32).i32_eq();
		w.if_(|w| {
			w.call(ruvarint).i32_wrap_i64().local_set(len);
			bail(w);
			// not enough input → unexpected-end.
			w.local_get(len)
				.global_get(g.input)
				.array_len()
				.global_get(g.pos)
				.i32_sub()
				.i32_gt_u();
			w.if_(|w| {
				w.i32(1).global_set(g.err);
				push_nothing(w);
				w.ret();
			});
			w.local_get(len)
				.array_new_default(types::T_BYTES)
				.local_set(bytes);
			w.i32(0).local_set(i);
			w.block("brk", |w| {
				w.loop_("lp", |w| {
					w.local_get(i).local_get(len).i32_ge_u().br_if("brk");
					w.local_get(bytes)
						.local_get(i)
						.call(rbyte)
						.array_set(types::T_BYTES);
					w.local_get(i).i32(1).i32_add().local_set(i);
					w.br("lp");
				});
			});
			w.i32(tag).local_get(bytes).struct_new(types::T_STR).ret();
		});
	};
	bytes_arm(&mut w, wt.s_string, types::TAG_STR);
	bytes_arm(&mut w, wt.s_bytes, types::TAG_BYTES);
	// nothing.
	w.local_get(vtag).i32(wt.s_nothing as i32).i32_eq();
	w.if_(|w| {
		push_nothing(w);
		w.ret();
	});
	// list: uvarint count, then each element under the inner schema (payload[0]).
	w.local_get(vtag).i32(wt.s_list as i32).i32_eq();
	w.if_(|w| {
		w.call(ruvarint).i32_wrap_i64().local_set(n);
		bail(w);
		payload_elem(w, 0);
		w.local_set(inner);
		w.local_get(n).array_new_default(va).local_set(out);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
				w.local_get(out).local_get(i);
				w.local_get(inner).call(self_idx).array_set(va);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		crate::helpers::list::mk_list(w, out);
		w.ret();
	});
	// tuple: a fixed number of fields (arity from the schema, no count on wire).
	w.local_get(vtag).i32(wt.s_tuple as i32).i32_eq();
	w.if_(|w| {
		payload_elem(w, 0);
		w.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(schemas);
		w.local_get(schemas).array_len().local_set(n);
		w.local_get(n).array_new_default(va).local_set(out);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
				w.local_get(out).local_get(i);
				w.local_get(schemas).local_get(i).array_get(va);
				w.call(self_idx).array_set(va);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.i32(types::TAG_TUPLE)
			.local_get(out)
			.struct_new(types::T_TUPLE)
			.ret();
	});
	// record: decode each field in schema (name-sorted) order; build the parallel
	// names/values arrays so the `$record` is name-sorted like `MakeRecord`.
	w.local_get(vtag).i32(wt.s_record as i32).i32_eq();
	w.if_(|w| {
		payload_elem(w, 0);
		w.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(fields);
		w.local_get(fields).array_len().local_set(n);
		w.local_get(n).array_new_default(va).local_set(names);
		w.local_get(n).array_new_default(va).local_set(values);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
				w.local_get(fields).local_get(i).array_get(va);
				w.ref_cast(types::T_TUPLE)
					.struct_get(types::T_TUPLE, 1)
					.local_set(ft);
				// names[i] = ft[0].
				w.local_get(names).local_get(i);
				w.local_get(ft).i32(0).array_get(va).array_set(va);
				// values[i] = dec(ft[1]).
				w.local_get(values).local_get(i);
				w.local_get(ft)
					.i32(1)
					.array_get(va)
					.call(self_idx)
					.array_set(va);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.i32(types::TAG_RECORD)
			.local_get(names)
			.local_get(values)
			.struct_new(types::T_RECORD)
			.ret();
	});
	// enum: register variants for inner `s-enum-ref`, then decode the variant.
	w.local_get(vtag).i32(wt.s_enum as i32).i32_eq();
	w.if_(|w| {
		payload_elem(w, 0);
		payload_elem(w, 1);
		w.call(ctxput).drop();
		payload_elem(w, 0);
		payload_elem(w, 1);
		w.call(dec_variant).ret();
	});
	// enum-ref: resolve registered variants by name (null → malformed).
	w.local_get(vtag).i32(wt.s_enum_ref as i32).i32_eq();
	w.if_(|w| {
		payload_elem(w, 0);
		w.call(ctxget).local_set(vars);
		w.local_get(vars).ref_is_null();
		w.if_(|w| {
			w.i32(5).global_set(g.err);
			push_nothing(w);
			w.ret();
		});
		payload_elem(w, 0);
		w.local_get(vars).call(dec_variant).ret();
	});
	// dict: uvarint count then (key, value) pairs in wire (canonical) order.
	// Rebuild the persistent trie by inserting each decoded pair — keys are
	// decoded before values, preserving the stream order.
	w.local_get(vtag).i32(wt.s_dict as i32).i32_eq();
	w.if_(|w| {
		w.call(ruvarint).i32_wrap_i64().local_set(n);
		bail(w);
		// inner = empty $dict { tag, root: null, next_seq: 0 }.
		w.i32(types::TAG_DICT)
			.ref_null(types::T_VALUE)
			.i32(0)
			.struct_new(types::T_DICT)
			.local_set(inner);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_u().br_if("brk");
				// inner = dict_insert(inner, dec(key-schema), dec(value-schema)).
				w.local_get(inner);
				payload_elem(w, 0);
				w.call(self_idx);
				payload_elem(w, 1);
				w.call(self_idx);
				w.call(dict_insert).local_set(inner);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.local_get(inner).ret();
	});
	// Fallthrough (malformed schema): nothing.
	push_nothing(&mut w);
	w.finish()
}

/// Build `__wire_result(value v) -> value`: wrap a decoded value in `ok v`, or in
/// the `wire-error` variant matching `g_err` (`err …`). Runs the trailing-bytes
/// check first: a fully-decoded value with input left over is `trailing-bytes`.
pub(crate) fn build_wire_result_fn(g: WireGlobals, lits: WireResultLits) -> Function {
	let va = types::T_VALARRAY;
	let mut w = Wat::new(1);
	let v = w.param(0);
	let e = w.local(types::value_ref());

	// Trailing-bytes check: only when otherwise-ok and input remains.
	w.global_get(g.err).i32_eqz();
	w.if_(|w| {
		w.global_get(g.pos)
			.global_get(g.input)
			.array_len()
			.i32_lt_u();
		w.if_(|w| {
			w.i32(4).global_set(g.err);
			w.global_get(g.input)
				.array_len()
				.global_get(g.pos)
				.i32_sub()
				.i64_extend_i32_u()
				.global_set(g.errval);
		});
	});
	// ok path.
	w.global_get(g.err).i32_eqz();
	w.if_(|w| {
		w.i32(types::TAG_VARIANT).i32(lits.ok_tag as i32);
		str_lit(w, lits.ok_name);
		w.local_get(v)
			.array_new_fixed(va, 1)
			.struct_new(types::T_VARIANT)
			.ret();
	});
	// err path: build the `wire-error` variant e for the error code (1..5), with
	// an `int` payload for invalid-tag / trailing-bytes.
	w.ref_null(types::T_VALUE).local_set(e);
	for code in 1..=5i32 {
		let (etag, ename) = lits.errors[(code - 1) as usize];
		let has_payload = code == 2 || code == 4;
		w.global_get(g.err).i32(code).i32_eq();
		w.if_(|w| {
			w.i32(types::TAG_VARIANT).i32(etag as i32);
			str_lit(w, ename);
			if has_payload {
				w.i32(types::TAG_INT)
					.global_get(g.errval)
					.struct_new(types::T_INT);
				w.array_new_fixed(va, 1);
			} else {
				w.array_new_fixed(va, 0);
			}
			w.struct_new(types::T_VARIANT).local_set(e);
		});
	}
	// err(e).
	w.i32(types::TAG_VARIANT).i32(lits.err_tag as i32);
	str_lit(&mut w, lits.err_name);
	w.local_get(e)
		.array_new_fixed(va, 1)
		.struct_new(types::T_VARIANT);
	w.finish()
}

/// Build `__wire_bcmp(value a, value b) -> i32`: lexicographic comparison of two
/// `$bytes`-backed values (each a `TAG_BYTES`/`$str`-shaped value), returning a
/// negative/zero/positive sign like `memcmp` with a length tie-break. Used to
/// sort dict entries by their encoded-key bytes (the canonical order).
pub(crate) fn build_wire_bcmp_fn() -> Function {
	let bytes = types::T_BYTES;
	let mut w = Wat::new(2);
	let (a, b) = (w.param(0), w.param(1));
	let ab = w.local(types::bytes_ref());
	let bb = w.local(types::bytes_ref());
	let la = w.local(ValType::I32);
	let lb = w.local(ValType::I32);
	let min = w.local(ValType::I32);
	let i = w.local(ValType::I32);

	w.local_get(a)
		.ref_cast(types::T_STR)
		.struct_get(types::T_STR, 1)
		.local_set(ab);
	w.local_get(b)
		.ref_cast(types::T_STR)
		.struct_get(types::T_STR, 1)
		.local_set(bb);
	w.local_get(ab).array_len().local_set(la);
	w.local_get(bb).array_len().local_set(lb);
	// min = min(la, lb).
	w.local_get(la).local_get(lb).i32_lt_u();
	w.if_result(
		ValType::I32,
		|w| {
			w.local_get(la);
		},
		|w| {
			w.local_get(lb);
		},
	);
	w.local_set(min);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(min).i32_ge_u().br_if("brk");
			// if a[i] != b[i]: return a[i] - b[i].
			w.local_get(ab).local_get(i).array_get_u(bytes);
			w.local_get(bb).local_get(i).array_get_u(bytes);
			w.i32_ne();
			w.if_(|w| {
				w.local_get(ab).local_get(i).array_get_u(bytes);
				w.local_get(bb).local_get(i).array_get_u(bytes);
				w.i32_sub().ret();
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	// common prefix equal: shorter sorts first.
	w.local_get(la).local_get(lb).i32_sub();
	w.finish()
}

/// Build `__wire_enc_dict(value schema, value val)`: encode a `dict` as a uvarint
/// count then `(key, value)` pairs sorted by encoded-key bytes (so logically-equal
/// dicts encode identically regardless of insertion order). Mirrors the VM's
/// `encode_in` dict arm. `schema` is the `s-dict` node (`payload[0]`=key schema,
/// `payload[1]`=value schema). Keys are encoded once into a captured `$bytes` via
/// a buffer rewind, then sorted with `__wire_bcmp` (insertion sort).
pub(crate) fn build_wire_enc_dict_fn(
	enc: u32,
	uvarint: u32,
	push: u32,
	bcmp: u32,
	dict_entries: u32,
	g: WireGlobals,
) -> Function {
	let va = types::T_VALARRAY;
	let bytes = types::T_BYTES;
	let mut w = Wat::new(2);
	let (schema, val) = (w.param(0), w.param(1));
	let lst = w.local(types::value_ref());
	let ksch = w.local(types::value_ref());
	let vsch = w.local(types::value_ref());
	let entries = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let pairs = w.local(types::valarray_ref());
	let i = w.local(ValType::I32);
	let entry = w.local(types::valarray_ref());
	let start = w.local(ValType::I32);
	let keylen = w.local(ValType::I32);
	let kb = w.local(types::bytes_ref());
	let gbuf = w.local(types::bytes_ref());
	let cur = w.local(types::value_ref());
	let curkey = w.local(types::value_ref());
	let j = w.local(ValType::I32);
	let m = w.local(ValType::I32);
	let kl = w.local(ValType::I32);

	// key/value schemas = schema.payload[0..2].
	let schema_payload = |w: &mut Wat, idx: i32| {
		w.local_get(schema)
			.ref_cast(types::T_VARIANT)
			.struct_get(types::T_VARIANT, 3);
		w.i32(idx).array_get(va);
	};
	// `(cast $tuple local).elems[idx]`.
	let tuple_elem = |w: &mut Wat, local: Local, idx: i32| {
		w.local_get(local)
			.ref_cast(types::T_TUPLE)
			.struct_get(types::T_TUPLE, 1);
		w.i32(idx).array_get(va);
	};

	schema_payload(&mut w, 0);
	w.local_set(ksch);
	schema_payload(&mut w, 1);
	w.local_set(vsch);
	// `__dict_entries` materializes the seq-ordered `(key, value)` list (this
	// helper then re-sorts the pairs into canonical key-byte order below).
	w.local_get(val).call(dict_entries).local_set(lst);
	w.local_get(lst)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(entries);
	w.local_get(lst)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 2)
		.local_set(n);
	w.local_get(n).array_new_default(va).local_set(pairs);
	// Pass 1: encode each key into `g_buf`, capture its bytes, rewind. pairs[i] =
	// tuple(key-bytes-value, value).
	w.i32(0).local_set(i);
	w.block("p1", |w| {
		w.loop_("p1l", |w| {
			w.local_get(i).local_get(n).i32_ge_u().br_if("p1");
			// entry = entries[i].elems.
			w.local_get(entries).local_get(i).array_get(va);
			w.ref_cast(types::T_TUPLE)
				.struct_get(types::T_TUPLE, 1)
				.local_set(entry);
			// start = g_len; enc(ksch, entry[0]); keylen = g_len - start.
			w.global_get(g.len).local_set(start);
			w.local_get(ksch);
			w.local_get(entry).i32(0).array_get(va);
			w.call(enc);
			w.global_get(g.len)
				.local_get(start)
				.i32_sub()
				.local_set(keylen);
			// kb = g_buf[start..start+keylen]; rewind g_len = start. (Loop, not
			// array.copy — see copy_loop_bytes; this runs once per dict entry.)
			w.local_get(keylen).array_new_default(bytes).local_set(kb);
			w.global_get(g.buf).ref_cast(bytes).local_set(gbuf);
			w.copy_loop_bytes(bytes, kb, None, gbuf, Some(start), keylen);
			w.local_get(start).global_set(g.len);
			// pairs[i] = tuple( $bytes-value(kb), entry[1] ).
			w.local_get(pairs).local_get(i);
			w.i32(types::TAG_TUPLE);
			w.i32(types::TAG_BYTES)
				.local_get(kb)
				.struct_new(types::T_STR);
			w.local_get(entry).i32(1).array_get(va);
			w.array_new_fixed(va, 2).struct_new(types::T_TUPLE);
			w.array_set(va);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("p1l");
		});
	});
	// Pass 2: insertion-sort pairs by key bytes (stable, ascending).
	w.i32(1).local_set(i);
	w.block("p2", |w| {
		w.loop_("p2l", |w| {
			w.local_get(i).local_get(n).i32_ge_u().br_if("p2");
			w.local_get(pairs).local_get(i).array_get(va).local_set(cur);
			tuple_elem(w, cur, 0);
			w.local_set(curkey);
			w.local_get(i).i32(1).i32_sub().local_set(j);
			// while j >= 0 && bcmp(pairs[j].key, curkey) > 0: pairs[j+1] = pairs[j]; j--.
			w.block("ins", |w| {
				w.loop_("insl", |w| {
					w.local_get(j).i32(0).i32_lt_s().br_if("ins");
					// key(j) = (cast $tuple pairs[j]).elems[0].
					w.local_get(pairs).local_get(j).array_get(va);
					w.ref_cast(types::T_TUPLE)
						.struct_get(types::T_TUPLE, 1)
						.i32(0)
						.array_get(va);
					w.local_get(curkey).call(bcmp);
					w.i32(0).i32_le_s().br_if("ins");
					// pairs[j+1] = pairs[j].
					w.local_get(pairs).local_get(j).i32(1).i32_add();
					w.local_get(pairs).local_get(j).array_get(va);
					w.array_set(va);
					w.local_get(j).i32(1).i32_sub().local_set(j);
					w.br("insl");
				});
			});
			// pairs[j+1] = cur.
			w.local_get(pairs).local_get(j).i32(1).i32_add();
			w.local_get(cur).array_set(va);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("p2l");
		});
	});
	// Pass 3: uvarint(count) then each sorted (key bytes, value).
	w.local_get(n).i64_extend_i32_u().call(uvarint);
	w.i32(0).local_set(i);
	w.block("p3", |w| {
		w.loop_("p3l", |w| {
			w.local_get(i).local_get(n).i32_ge_u().br_if("p3");
			// cur = pairs[i]; kb = key bytes; append each byte.
			w.local_get(pairs).local_get(i).array_get(va).local_set(cur);
			tuple_elem(w, cur, 0);
			w.ref_cast(types::T_STR)
				.struct_get(types::T_STR, 1)
				.local_set(kb);
			w.local_get(kb).array_len().local_set(kl);
			w.i32(0).local_set(m);
			w.block("byt", |w| {
				w.loop_("bytl", |w| {
					w.local_get(m).local_get(kl).i32_ge_u().br_if("byt");
					w.local_get(kb).local_get(m).array_get_u(bytes).call(push);
					w.local_get(m).i32(1).i32_add().local_set(m);
					w.br("bytl");
				});
			});
			// encode value under vsch.
			w.local_get(vsch);
			tuple_elem(w, cur, 1);
			w.call(enc);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("p3l");
		});
	});
	w.finish()
}
