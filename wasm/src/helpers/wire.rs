// `wire` codec helpers: the FNV-1a mixers and the recursive schema fingerprint
// (`__wire_mix_len`, `__wire_mix_str`, `__wire_fp`).

use wasm_encoder::*;

use crate::runtime::{WireGlobals, WireResultLits, WireTags, WIRE_FNV_PRIME};
use crate::types;

/// Build `__wire_mix_len(i64 h, i64 n) -> i64`: fold `mix_byte` over `n`'s 8
/// little-endian bytes (mirrors `vm::wire::mix_len`, where lengths are `u64` LE).
pub(crate) fn build_wire_mix_len_fn() -> Function {
	use Instruction as I;
	const H: u32 = 0;
	const N: u32 = 1;
	const I_: u32 = 2;
	let empty = wasm_encoder::BlockType::Empty;
	let mut b: Vec<Instruction> = vec![
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::I32Const(8),
		I::I32GeU,
		I::BrIf(1),
		// h = (h ^ ((n >> (i*8)) & 0xff)) * PRIME
		I::LocalGet(H),
		I::LocalGet(N),
		I::LocalGet(I_),
		I::I32Const(8),
		I::I32Mul,
		I::I64ExtendI32U,
		I::I64ShrU,
		I::I64Const(0xff),
		I::I64And,
		I::I64Xor,
		I::I64Const(WIRE_FNV_PRIME),
		I::I64Mul,
		I::LocalSet(H),
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::LocalGet(H),
	];
	let mut f = Function::new_with_locals_types(vec![ValType::I32]);
	for ins in b.drain(..) {
		f.instruction(&ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__wire_mix_str(i64 h, ref $value str) -> i64`: mix the string's byte
/// length (via `mix_len`) then each of its bytes (mirrors `vm::wire::mix_str`).
pub(crate) fn build_wire_mix_str_fn(mix_len: u32) -> Function {
	use Instruction as I;
	const H: u32 = 0;
	const S: u32 = 1;
	const BYTES: u32 = 2;
	const N: u32 = 3;
	const I_: u32 = 4;
	let empty = wasm_encoder::BlockType::Empty;
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let mut b: Vec<Instruction> = vec![
		// bytes = (cast $str s).field1
		I::LocalGet(S),
		I::RefCastNonNull(HeapType::Concrete(types::T_STR)),
		getf(types::T_STR, 1),
		I::LocalSet(BYTES),
		// n = array.len bytes; h = mix_len(h, n)
		I::LocalGet(BYTES),
		I::ArrayLen,
		I::LocalSet(N),
		I::LocalGet(H),
		I::LocalGet(N),
		I::I64ExtendI32U,
		I::Call(mix_len),
		I::LocalSet(H),
		// for i in 0..n: h = (h ^ byte) * PRIME
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(N),
		I::I32GeU,
		I::BrIf(1),
		I::LocalGet(H),
		I::LocalGet(BYTES),
		I::LocalGet(I_),
		I::ArrayGetU(types::T_BYTES),
		I::I64ExtendI32U,
		I::I64Xor,
		I::I64Const(WIRE_FNV_PRIME),
		I::I64Mul,
		I::LocalSet(H),
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::LocalGet(H),
	];
	let mut f = Function::new_with_locals_types(vec![types::bytes_ref(), ValType::I32, ValType::I32]);
	for ins in b.drain(..) {
		f.instruction(&ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__wire_fp(i64 h, ref $value schema) -> i64`: the recursive structural
/// fingerprint over a `wire-schema` value tree (mirrors `vm::wire::mix_schema`).
/// Dispatches on the schema node's `vtag`; each arm leads with a distinct kind
/// byte (1..13) so structurally-different schemas can't alias. `self_idx` is this
/// function's own wasm index (for recursion).
pub(crate) fn build_wire_fp_fn(self_idx: u32, mix_str: u32, mix_len: u32, w: WireTags) -> Function {
	use Instruction as I;
	const H: u32 = 0;
	const SCHEMA: u32 = 1;
	const VTAG: u32 = 2;
	const PAYLOAD: u32 = 3;
	const ELEMS: u32 = 4;
	const I_: u32 = 5;
	const N: u32 = 6;
	const PE: u32 = 7;
	const FIELDS: u32 = 8;
	const M: u32 = 9;
	const J: u32 = 10;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let mut b: Vec<Instruction> = Vec::new();
	// vtag = schema.variant_tag; payload = schema.payload.
	b.push(I::LocalGet(SCHEMA));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 1));
	b.push(I::LocalSet(VTAG));
	b.push(I::LocalGet(SCHEMA));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 3));
	b.push(I::LocalSet(PAYLOAD));
	// h = (h ^ kind) * PRIME, written back to H.
	let mix_byte = |b: &mut Vec<Instruction>, kind: i64| {
		b.push(I::LocalGet(H));
		b.push(I::I64Const(kind));
		b.push(I::I64Xor);
		b.push(I::I64Const(WIRE_FNV_PRIME));
		b.push(I::I64Mul);
		b.push(I::LocalSet(H));
	};
	// Push payload[idx] (a `$value`).
	let payload_elem = |b: &mut Vec<Instruction>, idx: i32| {
		b.push(I::LocalGet(PAYLOAD));
		b.push(I::I32Const(idx));
		b.push(I::ArrayGet(va));
	};
	// ELEMS = list-elems of payload[idx] (cast to `$list`, field 1).
	let elems_of = |b: &mut Vec<Instruction>, idx: i32, dst: u32| {
		b.push(I::LocalGet(PAYLOAD));
		b.push(I::I32Const(idx));
		b.push(I::ArrayGet(va));
		b.push(cast(types::T_LIST));
		b.push(getf(types::T_LIST, 1));
		b.push(I::LocalSet(dst));
	};
	// Scalar arm: `if vtag == t { mix_byte(kind); return h }`.
	let scalar = |b: &mut Vec<Instruction>, t: u32, kind: i64| {
		b.push(I::LocalGet(VTAG));
		b.push(I::I32Const(t as i32));
		b.push(I::I32Eq);
		b.push(I::If(empty));
		b.push(I::LocalGet(H));
		b.push(I::I64Const(kind));
		b.push(I::I64Xor);
		b.push(I::I64Const(WIRE_FNV_PRIME));
		b.push(I::I64Mul);
		b.push(I::Return);
		b.push(I::End);
	};
	scalar(&mut b, w.s_int, 1);
	scalar(&mut b, w.s_float, 2);
	scalar(&mut b, w.s_bool, 3);
	scalar(&mut b, w.s_string, 4);
	scalar(&mut b, w.s_bytes, 5);
	scalar(&mut b, w.s_duration, 6);
	scalar(&mut b, w.s_nothing, 7);
	// Open the `if vtag == t {` for a compound arm.
	let open = |b: &mut Vec<Instruction>, t: u32| {
		b.push(I::LocalGet(VTAG));
		b.push(I::I32Const(t as i32));
		b.push(I::I32Eq);
		b.push(I::If(empty));
	};
	// s-list: wire_fp(mix_byte(h, 8), inner=payload[0]).
	open(&mut b, w.s_list);
	mix_byte(&mut b, 8);
	b.push(I::LocalGet(H));
	payload_elem(&mut b, 0);
	b.push(I::Call(self_idx));
	b.push(I::Return);
	b.push(I::End);
	// s-dict: wire_fp(wire_fp(mix_byte(h, 12), k=payload[0]), v=payload[1]).
	open(&mut b, w.s_dict);
	mix_byte(&mut b, 12);
	b.push(I::LocalGet(H));
	payload_elem(&mut b, 0);
	b.push(I::Call(self_idx));
	b.push(I::LocalSet(H));
	b.push(I::LocalGet(H));
	payload_elem(&mut b, 1);
	b.push(I::Call(self_idx));
	b.push(I::Return);
	b.push(I::End);
	// s-enum-ref: mix_str(mix_byte(h, 13), qualified=payload[0]).
	open(&mut b, w.s_enum_ref);
	mix_byte(&mut b, 13);
	b.push(I::LocalGet(H));
	payload_elem(&mut b, 0);
	b.push(I::Call(mix_str));
	b.push(I::Return);
	b.push(I::End);
	// Fold `wire_fp` over the `$valarray` in local `arr`, length `N`, using loop
	// index `idx`; accumulates into `H`.
	let fold_fp = |b: &mut Vec<Instruction>, arr: u32, idx: u32| {
		b.push(I::I32Const(0));
		b.push(I::LocalSet(idx));
		b.push(I::Block(empty));
		b.push(I::Loop(empty));
		b.push(I::LocalGet(idx));
		b.push(I::LocalGet(N));
		b.push(I::I32GeU);
		b.push(I::BrIf(1));
		b.push(I::LocalGet(H));
		b.push(I::LocalGet(arr));
		b.push(I::LocalGet(idx));
		b.push(I::ArrayGet(va));
		b.push(I::Call(self_idx));
		b.push(I::LocalSet(H));
		b.push(I::LocalGet(idx));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(idx));
		b.push(I::Br(0));
		b.push(I::End); // loop
		b.push(I::End); // block
	};
	// h = mix_len(h, (i64) local N).
	let mix_len_n = |b: &mut Vec<Instruction>| {
		b.push(I::LocalGet(H));
		b.push(I::LocalGet(N));
		b.push(I::I64ExtendI32U);
		b.push(I::Call(mix_len));
		b.push(I::LocalSet(H));
	};
	// s-tuple: mix_len(mix_byte(h, 9), elems.len()); fold wire_fp over elems.
	open(&mut b, w.s_tuple);
	mix_byte(&mut b, 9);
	elems_of(&mut b, 0, ELEMS);
	b.push(I::LocalGet(ELEMS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	mix_len_n(&mut b);
	fold_fp(&mut b, ELEMS, I_);
	b.push(I::LocalGet(H));
	b.push(I::Return);
	b.push(I::End);
	// s-record: mix_len(mix_byte(h, 10), fields.len()); each field is a
	// `$tuple (name, schema)` — mix_str the name, recurse on the schema.
	open(&mut b, w.s_record);
	mix_byte(&mut b, 10);
	elems_of(&mut b, 0, ELEMS);
	b.push(I::LocalGet(ELEMS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	mix_len_n(&mut b);
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	// PE = (cast $tuple elems[i]).field1
	b.push(I::LocalGet(ELEMS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_TUPLE));
	b.push(getf(types::T_TUPLE, 1));
	b.push(I::LocalSet(PE));
	// h = mix_str(h, PE[0])
	b.push(I::LocalGet(H));
	b.push(I::LocalGet(PE));
	b.push(I::I32Const(0));
	b.push(I::ArrayGet(va));
	b.push(I::Call(mix_str));
	b.push(I::LocalSet(H));
	// h = wire_fp(h, PE[1])
	b.push(I::LocalGet(H));
	b.push(I::LocalGet(PE));
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(va));
	b.push(I::Call(self_idx));
	b.push(I::LocalSet(H));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	b.push(I::LocalGet(H));
	b.push(I::Return);
	b.push(I::End);
	// s-enum: mix_len(mix_str(mix_byte(h, 11), qualified), variants.len()); each
	// variant is a `$tuple (name, list-of-field-schemas)`.
	open(&mut b, w.s_enum);
	mix_byte(&mut b, 11);
	b.push(I::LocalGet(H));
	payload_elem(&mut b, 0);
	b.push(I::Call(mix_str));
	b.push(I::LocalSet(H));
	// ELEMS = variants list (payload[1] is a `$list`).
	b.push(I::LocalGet(PAYLOAD));
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_LIST));
	b.push(getf(types::T_LIST, 1));
	b.push(I::LocalSet(ELEMS));
	b.push(I::LocalGet(ELEMS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	mix_len_n(&mut b);
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty)); // over variants
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	// PE = (cast $tuple variants[i]).field1  (name, field-list)
	b.push(I::LocalGet(ELEMS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_TUPLE));
	b.push(getf(types::T_TUPLE, 1));
	b.push(I::LocalSet(PE));
	// h = mix_str(h, PE[0])  (variant name)
	b.push(I::LocalGet(H));
	b.push(I::LocalGet(PE));
	b.push(I::I32Const(0));
	b.push(I::ArrayGet(va));
	b.push(I::Call(mix_str));
	b.push(I::LocalSet(H));
	// FIELDS = (cast $list PE[1]).field1; M = len; h = mix_len(h, M)
	b.push(I::LocalGet(PE));
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_LIST));
	b.push(getf(types::T_LIST, 1));
	b.push(I::LocalSet(FIELDS));
	b.push(I::LocalGet(FIELDS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(M));
	b.push(I::LocalGet(H));
	b.push(I::LocalGet(M));
	b.push(I::I64ExtendI32U);
	b.push(I::Call(mix_len));
	b.push(I::LocalSet(H));
	// for j in 0..M: h = wire_fp(h, FIELDS[j])
	b.push(I::I32Const(0));
	b.push(I::LocalSet(J));
	b.push(I::Block(empty));
	b.push(I::Loop(empty)); // over fields
	b.push(I::LocalGet(J));
	b.push(I::LocalGet(M));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	b.push(I::LocalGet(H));
	b.push(I::LocalGet(FIELDS));
	b.push(I::LocalGet(J));
	b.push(I::ArrayGet(va));
	b.push(I::Call(self_idx));
	b.push(I::LocalSet(H));
	b.push(I::LocalGet(J));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(J));
	b.push(I::Br(0));
	b.push(I::End); // inner loop
	b.push(I::End); // inner block
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // outer loop
	b.push(I::End); // outer block
	b.push(I::LocalGet(H));
	b.push(I::Return);
	b.push(I::End);
	// Fallthrough (malformed schema): return h unchanged.
	b.push(I::LocalGet(H));
	let locals = vec![
		ValType::I32,          // VTAG
		types::valarray_ref(), // PAYLOAD
		types::valarray_ref(), // ELEMS
		ValType::I32,          // I_
		ValType::I32,          // N
		types::valarray_ref(), // PE
		types::valarray_ref(), // FIELDS
		ValType::I32,          // M
		ValType::I32,          // J
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
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

/// Finish a `Function` from its instruction buffer + local declarations.
fn finish(locals: Vec<ValType>, body: &[Instruction]) -> Function {
	let mut f = Function::new_with_locals_types(locals);
	for ins in body {
		f.instruction(ins);
	}
	f.instruction(&Instruction::End);
	f
}

/// Read `$value` field `f` of struct type `t` (a `StructGet`).
fn getf(t: u32, f: u32) -> Instruction<'static> {
	Instruction::StructGet {
		struct_type_index: t,
		field_index: f,
	}
}

fn cast(t: u32) -> Instruction<'static> {
	Instruction::RefCastNonNull(HeapType::Concrete(t))
}

/// Build `__wire_push(i32 b)`: append `b` to the encode buffer `g_buf`, growing
/// it (doubling) when full. `g_buf` is pre-initialized non-null at the call site,
/// so `array.len`/`array.set` never see null.
pub(crate) fn build_wire_push_fn(g: WireGlobals) -> Function {
	use Instruction as I;
	const B: u32 = 0;
	const NEW: u32 = 1;
	let empty = BlockType::Empty;
	let bytes = types::T_BYTES;
	let b: Vec<Instruction> = vec![
		// if g_len >= array.len(g_buf): grow.
		I::GlobalGet(g.len),
		I::GlobalGet(g.buf),
		I::ArrayLen,
		I::I32GeU,
		I::If(empty),
		// NEW = array.new_default $bytes (cap * 2).
		I::GlobalGet(g.buf),
		I::ArrayLen,
		I::I32Const(1),
		I::I32Shl,
		I::ArrayNewDefault(bytes),
		I::LocalSet(NEW),
		// NEW[0..g_len] = g_buf[0..g_len].
		I::LocalGet(NEW),
		I::I32Const(0),
		I::GlobalGet(g.buf),
		I::I32Const(0),
		I::GlobalGet(g.len),
		I::ArrayCopy {
			array_type_index_dst: bytes,
			array_type_index_src: bytes,
		},
		I::LocalGet(NEW),
		I::GlobalSet(g.buf),
		I::End,
		// g_buf[g_len] = b.
		I::GlobalGet(g.buf),
		I::GlobalGet(g.len),
		I::LocalGet(B),
		I::ArraySet(bytes),
		// g_len += 1.
		I::GlobalGet(g.len),
		I::I32Const(1),
		I::I32Add,
		I::GlobalSet(g.len),
	];
	finish(vec![types::bytes_ref()], &b)
}

/// Build `__wire_uvarint(i64 v)`: write `v` as an LEB128 unsigned varint via
/// `__wire_push` (mirrors `vm::wire::write_uvarint`).
pub(crate) fn build_wire_uvarint_fn(push: u32) -> Function {
	use Instruction as I;
	const V: u32 = 0;
	const BYTE: u32 = 1;
	let empty = BlockType::Empty;
	let b: Vec<Instruction> = vec![
		I::Loop(empty),
		// byte = v & 0x7f.
		I::LocalGet(V),
		I::I64Const(0x7f),
		I::I64And,
		I::I32WrapI64,
		I::LocalSet(BYTE),
		// v >>= 7 (unsigned).
		I::LocalGet(V),
		I::I64Const(7),
		I::I64ShrU,
		I::LocalSet(V),
		// if v == 0: push(byte); return.
		I::LocalGet(V),
		I::I64Eqz,
		I::If(empty),
		I::LocalGet(BYTE),
		I::Call(push),
		I::Return,
		I::End,
		// else push(byte | 0x80); continue.
		I::LocalGet(BYTE),
		I::I32Const(0x80),
		I::I32Or,
		I::Call(push),
		I::Br(0),
		I::End, // loop
	];
	finish(vec![ValType::I32], &b)
}

/// Build `__wire_ctxput(value qualified, value variants) -> value`: register the
/// inline enum `(qualified, variants)` in the recursive-enum registry `g_ctx`
/// (append, growing by doubling), returning `variants` for convenience.
pub(crate) fn build_wire_ctxput_fn(g: WireGlobals) -> Function {
	use Instruction as I;
	const QUAL: u32 = 0;
	const VARS: u32 = 1;
	const NEW: u32 = 2;
	let empty = BlockType::Empty;
	let va = types::T_VALARRAY;
	let b: Vec<Instruction> = vec![
		// if g_ctxlen >= array.len(g_ctx): grow.
		I::GlobalGet(g.ctxlen),
		I::GlobalGet(g.ctx),
		I::ArrayLen,
		I::I32GeU,
		I::If(empty),
		I::GlobalGet(g.ctx),
		I::ArrayLen,
		I::I32Const(1),
		I::I32Shl,
		I::ArrayNewDefault(va),
		I::LocalSet(NEW),
		I::LocalGet(NEW),
		I::I32Const(0),
		I::GlobalGet(g.ctx),
		I::I32Const(0),
		I::GlobalGet(g.ctxlen),
		I::ArrayCopy {
			array_type_index_dst: va,
			array_type_index_src: va,
		},
		I::LocalGet(NEW),
		I::GlobalSet(g.ctx),
		I::End,
		// g_ctx[g_ctxlen] = tuple(qualified, variants).
		I::GlobalGet(g.ctx),
		I::GlobalGet(g.ctxlen),
		I::I32Const(types::TAG_TUPLE),
		I::LocalGet(QUAL),
		I::LocalGet(VARS),
		I::ArrayNewFixed {
			array_type_index: va,
			array_size: 2,
		},
		I::StructNew(types::T_TUPLE),
		I::ArraySet(va),
		// g_ctxlen += 1.
		I::GlobalGet(g.ctxlen),
		I::I32Const(1),
		I::I32Add,
		I::GlobalSet(g.ctxlen),
		// return variants.
		I::LocalGet(VARS),
	];
	finish(vec![types::valarray_ref()], &b)
}

/// Build `__wire_ctxget(value qualified) -> value`: linear-scan `g_ctx` for the
/// entry whose name `__eq` `qualified`, returning its variants `$list` (or null
/// if unregistered — the decoder treats null as a malformed back-reference).
pub(crate) fn build_wire_ctxget_fn(eq: u32, g: WireGlobals) -> Function {
	use Instruction as I;
	const QUAL: u32 = 0;
	const I_: u32 = 1;
	const ENTRY: u32 = 2;
	let empty = BlockType::Empty;
	let va = types::T_VALARRAY;
	let b: Vec<Instruction> = vec![
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::GlobalGet(g.ctxlen),
		I::I32GeU,
		I::BrIf(1),
		// ENTRY = (cast $tuple g_ctx[i]).elems.
		I::GlobalGet(g.ctx),
		I::LocalGet(I_),
		I::ArrayGet(va),
		cast(types::T_TUPLE),
		getf(types::T_TUPLE, 1),
		I::LocalSet(ENTRY),
		// if __eq(ENTRY[0], qualified): return ENTRY[1].
		I::LocalGet(ENTRY),
		I::I32Const(0),
		I::ArrayGet(va),
		I::LocalGet(QUAL),
		I::Call(eq),
		I::If(empty),
		I::LocalGet(ENTRY),
		I::I32Const(1),
		I::ArrayGet(va),
		I::Return,
		I::End,
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		// not found: null.
		I::RefNull(HeapType::Concrete(types::T_VALUE)),
	];
	finish(vec![ValType::I32, types::valarray_ref()], &b)
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
	w: WireTags,
) -> Function {
	use Instruction as I;
	const SCHEMA: u32 = 0;
	const VAL: u32 = 1;
	const VTAG: u32 = 2;
	const PAYLOAD: u32 = 3;
	const N: u32 = 4;
	const I_: u32 = 5;
	const ELEMS: u32 = 6;
	const SCHEMAS: u32 = 7;
	const BITS: u32 = 8;
	const BYTES: u32 = 9;
	let empty = BlockType::Empty;
	let va = types::T_VALARRAY;
	let mut b: Vec<Instruction> = Vec::new();
	// vtag = schema.variant_tag; payload = schema.payload.
	b.push(I::LocalGet(SCHEMA));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 1));
	b.push(I::LocalSet(VTAG));
	b.push(I::LocalGet(SCHEMA));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 3));
	b.push(I::LocalSet(PAYLOAD));
	let open = |b: &mut Vec<Instruction>, t: u32| {
		b.push(I::LocalGet(VTAG));
		b.push(I::I32Const(t as i32));
		b.push(I::I32Eq);
		b.push(I::If(empty));
	};
	let payload_elem = |b: &mut Vec<Instruction>, idx: i32| {
		b.push(I::LocalGet(PAYLOAD));
		b.push(I::I32Const(idx));
		b.push(I::ArrayGet(va));
	};
	// int / duration: uvarint(zigzag(unbox-i64 val)).
	let int_arm = |b: &mut Vec<Instruction>, t: u32| {
		open(b, t);
		// (n << 1) ^ (n >> 63), recomputing `n` (cheap) rather than spilling.
		b.push(I::LocalGet(VAL));
		b.push(cast(types::T_INT));
		b.push(getf(types::T_INT, 1));
		b.push(I::I64Const(1));
		b.push(I::I64Shl);
		b.push(I::LocalGet(VAL));
		b.push(cast(types::T_INT));
		b.push(getf(types::T_INT, 1));
		b.push(I::I64Const(63));
		b.push(I::I64ShrS);
		b.push(I::I64Xor);
		b.push(I::Call(uvarint));
		b.push(I::Return);
		b.push(I::End);
	};
	int_arm(&mut b, w.s_int);
	int_arm(&mut b, w.s_duration);
	// float: 8 little-endian bytes of the IEEE-754 bit pattern.
	open(&mut b, w.s_float);
	b.push(I::LocalGet(VAL));
	b.push(cast(types::T_FLOAT));
	b.push(getf(types::T_FLOAT, 1));
	b.push(I::I64ReinterpretF64);
	b.push(I::LocalSet(BITS));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(8));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	b.push(I::LocalGet(BITS));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(8));
	b.push(I::I32Mul);
	b.push(I::I64ExtendI32U);
	b.push(I::I64ShrU);
	b.push(I::I64Const(0xff));
	b.push(I::I64And);
	b.push(I::I32WrapI64);
	b.push(I::Call(push));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	b.push(I::Return);
	b.push(I::End);
	// bool: one byte.
	open(&mut b, w.s_bool);
	b.push(I::LocalGet(VAL));
	b.push(cast(types::T_BOOL));
	b.push(getf(types::T_BOOL, 1));
	b.push(I::Call(push));
	b.push(I::Return);
	b.push(I::End);
	// string / bytes: uvarint(len) then the raw bytes (both reuse `$str` shape).
	let bytes_arm = |b: &mut Vec<Instruction>, t: u32| {
		open(b, t);
		b.push(I::LocalGet(VAL));
		b.push(cast(types::T_STR));
		b.push(getf(types::T_STR, 1));
		b.push(I::LocalSet(BYTES));
		b.push(I::LocalGet(BYTES));
		b.push(I::ArrayLen);
		b.push(I::LocalSet(N));
		b.push(I::LocalGet(N));
		b.push(I::I64ExtendI32U);
		b.push(I::Call(uvarint));
		b.push(I::I32Const(0));
		b.push(I::LocalSet(I_));
		b.push(I::Block(empty));
		b.push(I::Loop(empty));
		b.push(I::LocalGet(I_));
		b.push(I::LocalGet(N));
		b.push(I::I32GeU);
		b.push(I::BrIf(1));
		b.push(I::LocalGet(BYTES));
		b.push(I::LocalGet(I_));
		b.push(I::ArrayGetU(types::T_BYTES));
		b.push(I::Call(push));
		b.push(I::LocalGet(I_));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(I_));
		b.push(I::Br(0));
		b.push(I::End); // loop
		b.push(I::End); // block
		b.push(I::Return);
		b.push(I::End);
	};
	bytes_arm(&mut b, w.s_string);
	bytes_arm(&mut b, w.s_bytes);
	// nothing: zero bytes.
	open(&mut b, w.s_nothing);
	b.push(I::Return);
	b.push(I::End);
	// list: uvarint(count) then each element under the inner schema (payload[0]).
	open(&mut b, w.s_list);
	b.push(I::LocalGet(VAL));
	b.push(cast(types::T_LIST));
	b.push(getf(types::T_LIST, 1));
	b.push(I::LocalSet(ELEMS));
	b.push(I::LocalGet(ELEMS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::LocalGet(N));
	b.push(I::I64ExtendI32U);
	b.push(I::Call(uvarint));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	payload_elem(&mut b, 0);
	b.push(I::LocalGet(ELEMS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(I::Call(self_idx));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	b.push(I::Return);
	b.push(I::End);
	// tuple: each field in order; schemas = list-elems of payload[0], values =
	// the `$tuple`'s own elems (arity matches, no count on the wire).
	open(&mut b, w.s_tuple);
	payload_elem(&mut b, 0);
	b.push(cast(types::T_LIST));
	b.push(getf(types::T_LIST, 1));
	b.push(I::LocalSet(SCHEMAS));
	b.push(I::LocalGet(VAL));
	b.push(cast(types::T_TUPLE));
	b.push(getf(types::T_TUPLE, 1));
	b.push(I::LocalSet(ELEMS));
	b.push(I::LocalGet(SCHEMAS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	b.push(I::LocalGet(SCHEMAS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(I::LocalGet(ELEMS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(I::Call(self_idx));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	b.push(I::Return);
	b.push(I::End);
	// record: field schemas = list of `$tuple(name, schema)` in payload[0],
	// canonical (name-sorted) order; the `$record`'s values array is the same
	// order, so encode positionally (mirrors VM's per-name lookup).
	open(&mut b, w.s_record);
	payload_elem(&mut b, 0);
	b.push(cast(types::T_LIST));
	b.push(getf(types::T_LIST, 1));
	b.push(I::LocalSet(SCHEMAS));
	b.push(I::LocalGet(VAL));
	b.push(cast(types::T_RECORD));
	b.push(getf(types::T_RECORD, 2));
	b.push(I::LocalSet(ELEMS));
	b.push(I::LocalGet(SCHEMAS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	// schema = (cast $tuple SCHEMAS[i]).elems[1].
	b.push(I::LocalGet(SCHEMAS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_TUPLE));
	b.push(getf(types::T_TUPLE, 1));
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(va));
	b.push(I::LocalGet(ELEMS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(I::Call(self_idx));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	b.push(I::Return);
	b.push(I::End);
	// enum: register `(qualified, variants)` for any inner `s-enum-ref`, then
	// encode the variant tag + payload.
	open(&mut b, w.s_enum);
	payload_elem(&mut b, 0);
	payload_elem(&mut b, 1);
	b.push(I::Call(ctxput));
	b.push(I::Drop);
	payload_elem(&mut b, 1);
	b.push(I::LocalGet(VAL));
	b.push(I::Call(enc_variant));
	b.push(I::Return);
	b.push(I::End);
	// enum-ref: resolve the registered variants by name, then encode.
	open(&mut b, w.s_enum_ref);
	payload_elem(&mut b, 0);
	b.push(I::Call(ctxget));
	b.push(I::LocalGet(VAL));
	b.push(I::Call(enc_variant));
	b.push(I::Return);
	b.push(I::End);
	// dict: canonical key-sorted encode (its own helper — needs scratch state for
	// the key bytes + sort).
	open(&mut b, w.s_dict);
	b.push(I::LocalGet(SCHEMA));
	b.push(I::LocalGet(VAL));
	b.push(I::Call(enc_dict));
	b.push(I::Return);
	b.push(I::End);
	// Fallthrough (unreachable for well-typed values): emit nothing.
	let locals = vec![
		ValType::I32,          // VTAG
		types::valarray_ref(), // PAYLOAD
		ValType::I32,          // N
		ValType::I32,          // I_
		types::valarray_ref(), // ELEMS
		types::valarray_ref(), // SCHEMAS
		ValType::I64,          // BITS
		types::bytes_ref(),    // BYTES
	];
	finish(locals, &b)
}

/// Build `__wire_enc_variant(value variants, value val)`: write the variant's
/// declaration-index tag (a uvarint) then encode each payload field under its
/// schema (mirrors `vm::wire::encode_variant`). `variants` is the enum's variant
/// `$list` (`$tuple(name, field-schema-list)` per variant); the value's own
/// `vtag` is the wire tag and the index into `variants`.
pub(crate) fn build_wire_enc_variant_fn(enc: u32, uvarint: u32) -> Function {
	use Instruction as I;
	const VARIANTS: u32 = 0;
	const VAL: u32 = 1;
	const VARELEMS: u32 = 2;
	const VVTAG: u32 = 3;
	const FSCHEMAS: u32 = 4;
	const PV: u32 = 5;
	const M: u32 = 6;
	const J: u32 = 7;
	let empty = BlockType::Empty;
	let va = types::T_VALARRAY;
	let b: Vec<Instruction> = vec![
		// VARELEMS = (cast $list variants).elems.
		I::LocalGet(VARIANTS),
		cast(types::T_LIST),
		getf(types::T_LIST, 1),
		I::LocalSet(VARELEMS),
		// VVTAG = (cast $variant val).variant_tag; uvarint(VVTAG).
		I::LocalGet(VAL),
		cast(types::T_VARIANT),
		getf(types::T_VARIANT, 1),
		I::LocalSet(VVTAG),
		I::LocalGet(VVTAG),
		I::I64ExtendI32U,
		I::Call(uvarint),
		// FSCHEMAS = (cast $list (cast $tuple VARELEMS[VVTAG]).elems[1]).elems.
		I::LocalGet(VARELEMS),
		I::LocalGet(VVTAG),
		I::ArrayGet(va),
		cast(types::T_TUPLE),
		getf(types::T_TUPLE, 1),
		I::I32Const(1),
		I::ArrayGet(va),
		cast(types::T_LIST),
		getf(types::T_LIST, 1),
		I::LocalSet(FSCHEMAS),
		// PV = (cast $variant val).payload; M = len(FSCHEMAS).
		I::LocalGet(VAL),
		cast(types::T_VARIANT),
		getf(types::T_VARIANT, 3),
		I::LocalSet(PV),
		I::LocalGet(FSCHEMAS),
		I::ArrayLen,
		I::LocalSet(M),
		// for j in 0..M: enc(FSCHEMAS[j], PV[j]).
		I::I32Const(0),
		I::LocalSet(J),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(J),
		I::LocalGet(M),
		I::I32GeU,
		I::BrIf(1),
		I::LocalGet(FSCHEMAS),
		I::LocalGet(J),
		I::ArrayGet(va),
		I::LocalGet(PV),
		I::LocalGet(J),
		I::ArrayGet(va),
		I::Call(enc),
		I::LocalGet(J),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(J),
		I::Br(0),
		I::End, // loop
		I::End, // block
	];
	let locals = vec![
		types::valarray_ref(), // VARELEMS
		ValType::I32,          // VVTAG
		types::valarray_ref(), // FSCHEMAS
		types::valarray_ref(), // PV
		ValType::I32,          // M
		ValType::I32,          // J
	];
	finish(locals, &b)
}

/// Push a fresh `$str` for an interned data-segment literal `(off, len)`.
fn str_lit(b: &mut Vec<Instruction>, (off, len): (u32, u32)) {
	b.push(Instruction::I32Const(types::TAG_STR));
	b.push(Instruction::I32Const(off as i32));
	b.push(Instruction::I32Const(len as i32));
	b.push(Instruction::ArrayNewData {
		array_type_index: types::T_BYTES,
		array_data_index: 0,
	});
	b.push(Instruction::StructNew(types::T_STR));
}

fn push_nothing(b: &mut Vec<Instruction>) {
	b.push(Instruction::I32Const(types::TAG_NOTHING));
	b.push(Instruction::StructNew(types::T_VALUE));
}

/// Build `__wire_rbyte() -> i32`: read one input byte, advancing `g_pos`. Once
/// `g_err` is set (or the cursor is at end) it's a no-op returning 0, so the
/// first error wins and over-reads don't trap.
pub(crate) fn build_wire_rbyte_fn(g: WireGlobals) -> Function {
	use Instruction as I;
	const BYTE: u32 = 0;
	let empty = BlockType::Empty;
	let b: Vec<Instruction> = vec![
		// already failed: preserve the first error, return 0.
		I::GlobalGet(g.err),
		I::If(empty),
		I::I32Const(0),
		I::Return,
		I::End,
		// out of bytes: unexpected-end.
		I::GlobalGet(g.pos),
		I::GlobalGet(g.input),
		I::ArrayLen,
		I::I32GeU,
		I::If(empty),
		I::I32Const(1),
		I::GlobalSet(g.err),
		I::I32Const(0),
		I::Return,
		I::End,
		// b = g_in[g_pos]; g_pos += 1.
		I::GlobalGet(g.input),
		I::GlobalGet(g.pos),
		I::ArrayGetU(types::T_BYTES),
		I::LocalSet(BYTE),
		I::GlobalGet(g.pos),
		I::I32Const(1),
		I::I32Add,
		I::GlobalSet(g.pos),
		I::LocalGet(BYTE),
	];
	finish(vec![ValType::I32], &b)
}

/// Build `__wire_ruvarint() -> i64`: read an LEB128 unsigned varint (10-byte cap;
/// overlong/unterminated → `g_err = 5` malformed). Mirrors `vm::wire::read_uvarint`.
pub(crate) fn build_wire_ruvarint_fn(rbyte: u32, g: WireGlobals) -> Function {
	use Instruction as I;
	const RESULT: u32 = 0;
	const SHIFT: u32 = 1;
	const I_: u32 = 2;
	const BYTE: u32 = 3;
	let empty = BlockType::Empty;
	let b: Vec<Instruction> = vec![
		I::I64Const(0),
		I::LocalSet(RESULT),
		I::I32Const(0),
		I::LocalSet(SHIFT),
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		// 10 bytes consumed without terminator → malformed (handled after block).
		I::LocalGet(I_),
		I::I32Const(10),
		I::I32GeU,
		I::BrIf(1),
		// byte = rbyte(); bail if that errored.
		I::Call(rbyte),
		I::LocalSet(BYTE),
		I::GlobalGet(g.err),
		I::If(empty),
		I::I64Const(0),
		I::Return,
		I::End,
		// on the 10th byte (i==9) only the low bit is valid for a 64-bit int.
		I::LocalGet(I_),
		I::I32Const(9),
		I::I32Eq,
		I::LocalGet(BYTE),
		I::I32Const(1),
		I::I32GtU,
		I::I32And,
		I::If(empty),
		I::I32Const(5),
		I::GlobalSet(g.err),
		I::I64Const(0),
		I::Return,
		I::End,
		// result |= (byte & 0x7f) << shift.
		I::LocalGet(RESULT),
		I::LocalGet(BYTE),
		I::I32Const(0x7f),
		I::I32And,
		I::I64ExtendI32U,
		I::LocalGet(SHIFT),
		I::I64ExtendI32U,
		I::I64Shl,
		I::I64Or,
		I::LocalSet(RESULT),
		// high bit clear → done.
		I::LocalGet(BYTE),
		I::I32Const(0x80),
		I::I32And,
		I::I32Eqz,
		I::If(empty),
		I::LocalGet(RESULT),
		I::Return,
		I::End,
		I::LocalGet(SHIFT),
		I::I32Const(7),
		I::I32Add,
		I::LocalSet(SHIFT),
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		// loop exhausted: malformed.
		I::I32Const(5),
		I::GlobalSet(g.err),
		I::I64Const(0),
	];
	finish(
		vec![ValType::I64, ValType::I32, ValType::I32, ValType::I32],
		&b,
	)
}

/// Build `__wire_disp(value qualified, value varname) -> value`: rebuild a
/// decoded variant's display name `"<bare-enum>.<variant>"` (bare = the qualified
/// name after its last `.`), so `to-string`/the host formatter render it like a
/// literally-constructed variant. Equality/pattern-match use the `vtag`, not this.
pub(crate) fn build_wire_disp_fn(bytesconcat: u32) -> Function {
	use Instruction as I;
	const QUAL: u32 = 0;
	const VARNAME: u32 = 1;
	const QB: u32 = 2;
	const N: u32 = 3;
	const LAST: u32 = 4;
	const I_: u32 = 5;
	const START: u32 = 6;
	const BARELEN: u32 = 7;
	const BARE: u32 = 8;
	let empty = BlockType::Empty;
	let bytes = types::T_BYTES;
	let b: Vec<Instruction> = vec![
		// QB = qualified bytes; N = len.
		I::LocalGet(QUAL),
		cast(types::T_STR),
		getf(types::T_STR, 1),
		I::LocalSet(QB),
		I::LocalGet(QB),
		I::ArrayLen,
		I::LocalSet(N),
		// LAST = index of the last '.' (46), or -1.
		I::I32Const(-1),
		I::LocalSet(LAST),
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(N),
		I::I32GeU,
		I::BrIf(1),
		I::LocalGet(QB),
		I::LocalGet(I_),
		I::ArrayGetU(bytes),
		I::I32Const(46),
		I::I32Eq,
		I::If(empty),
		I::LocalGet(I_),
		I::LocalSet(LAST),
		I::End,
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		// START = LAST + 1; BARELEN = N - START.
		I::LocalGet(LAST),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(START),
		I::LocalGet(N),
		I::LocalGet(START),
		I::I32Sub,
		I::LocalSet(BARELEN),
		// BARE = QB[START..N].
		I::LocalGet(BARELEN),
		I::ArrayNewDefault(bytes),
		I::LocalSet(BARE),
		I::LocalGet(BARE),
		I::I32Const(0),
		I::LocalGet(QB),
		I::LocalGet(START),
		I::LocalGet(BARELEN),
		I::ArrayCopy {
			array_type_index_dst: bytes,
			array_type_index_src: bytes,
		},
		// result = $str( (BARE ++ ".") ++ varname-bytes ).
		I::I32Const(types::TAG_STR),
		I::LocalGet(BARE),
		I::I32Const(46),
		I::ArrayNewFixed {
			array_type_index: bytes,
			array_size: 1,
		},
		I::Call(bytesconcat),
		I::LocalGet(VARNAME),
		cast(types::T_STR),
		getf(types::T_STR, 1),
		I::Call(bytesconcat),
		I::StructNew(types::T_STR),
	];
	let locals = vec![
		types::bytes_ref(), // QB
		ValType::I32,       // N
		ValType::I32,       // LAST
		ValType::I32,       // I_
		ValType::I32,       // START
		ValType::I32,       // BARELEN
		types::bytes_ref(), // BARE
	];
	finish(locals, &b)
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
	use Instruction as I;
	const QUAL: u32 = 0;
	const VARIANTS: u32 = 1;
	const TAG: u32 = 2;
	const VARELEMS: u32 = 3;
	const M: u32 = 4;
	const IDX: u32 = 5;
	const TUP: u32 = 6;
	const NAME: u32 = 7;
	const FSL: u32 = 8;
	const K: u32 = 9;
	const J: u32 = 10;
	const PAYLOAD: u32 = 11;
	const DISP: u32 = 12;
	let empty = BlockType::Empty;
	let va = types::T_VALARRAY;
	let mut b: Vec<Instruction> = Vec::new();
	// TAG = ruvarint(); bail on read failure.
	b.push(I::Call(ruvarint));
	b.push(I::LocalSet(TAG));
	b.push(I::GlobalGet(g.err));
	b.push(I::If(empty));
	push_nothing(&mut b);
	b.push(I::Return);
	b.push(I::End);
	// VARELEMS = (cast $list variants).elems; M = len.
	b.push(I::LocalGet(VARIANTS));
	b.push(cast(types::T_LIST));
	b.push(getf(types::T_LIST, 1));
	b.push(I::LocalSet(VARELEMS));
	b.push(I::LocalGet(VARELEMS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(M));
	// if TAG < 0 || TAG >= M: invalid-tag.
	b.push(I::LocalGet(TAG));
	b.push(I::I64Const(0));
	b.push(I::I64LtS);
	b.push(I::LocalGet(TAG));
	b.push(I::LocalGet(M));
	b.push(I::I64ExtendI32U);
	b.push(I::I64GeS);
	b.push(I::I32Or);
	b.push(I::If(empty));
	b.push(I::I32Const(2));
	b.push(I::GlobalSet(g.err));
	b.push(I::LocalGet(TAG));
	b.push(I::GlobalSet(g.errval));
	push_nothing(&mut b);
	b.push(I::Return);
	b.push(I::End);
	b.push(I::LocalGet(TAG));
	b.push(I::I32WrapI64);
	b.push(I::LocalSet(IDX));
	// TUP = (cast $tuple VARELEMS[IDX]).elems; NAME = TUP[0]; FSL = (cast $list
	// TUP[1]).elems; K = len.
	b.push(I::LocalGet(VARELEMS));
	b.push(I::LocalGet(IDX));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_TUPLE));
	b.push(getf(types::T_TUPLE, 1));
	b.push(I::LocalSet(TUP));
	b.push(I::LocalGet(TUP));
	b.push(I::I32Const(0));
	b.push(I::ArrayGet(va));
	b.push(I::LocalSet(NAME));
	b.push(I::LocalGet(TUP));
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_LIST));
	b.push(getf(types::T_LIST, 1));
	b.push(I::LocalSet(FSL));
	b.push(I::LocalGet(FSL));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(K));
	// PAYLOAD = $valarray(K); for j: PAYLOAD[j] = dec(FSL[j]).
	b.push(I::LocalGet(K));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(PAYLOAD));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(J));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(J));
	b.push(I::LocalGet(K));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	b.push(I::LocalGet(PAYLOAD));
	b.push(I::LocalGet(J));
	b.push(I::LocalGet(FSL));
	b.push(I::LocalGet(J));
	b.push(I::ArrayGet(va));
	b.push(I::Call(dec));
	b.push(I::ArraySet(va));
	b.push(I::LocalGet(J));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(J));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
								 // DISP = disp(qualified, NAME); build $variant{tag, IDX, DISP, PAYLOAD}.
	b.push(I::LocalGet(QUAL));
	b.push(I::LocalGet(NAME));
	b.push(I::Call(disp));
	b.push(I::LocalSet(DISP));
	b.push(I::I32Const(types::TAG_VARIANT));
	b.push(I::LocalGet(IDX));
	b.push(I::LocalGet(DISP));
	b.push(I::LocalGet(PAYLOAD));
	b.push(I::StructNew(types::T_VARIANT));
	let locals = vec![
		ValType::I64,          // TAG
		types::valarray_ref(), // VARELEMS
		ValType::I32,          // M
		ValType::I32,          // IDX
		types::valarray_ref(), // TUP
		types::value_ref(),    // NAME
		types::valarray_ref(), // FSL
		ValType::I32,          // K
		ValType::I32,          // J
		types::valarray_ref(), // PAYLOAD
		types::value_ref(),    // DISP
	];
	finish(locals, &b)
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
	g: WireGlobals,
	w: WireTags,
) -> Function {
	use Instruction as I;
	const SCHEMA: u32 = 0;
	const VTAG: u32 = 1;
	const PAYLOAD: u32 = 2;
	const N: u32 = 3;
	const I_: u32 = 4;
	const SCHEMAS: u32 = 5;
	const OUT: u32 = 6;
	const NAMES: u32 = 7;
	const VALUES: u32 = 8;
	const FIELDS: u32 = 9;
	const FT: u32 = 10;
	const INNER: u32 = 11;
	const U: u32 = 12;
	const BITS: u32 = 13;
	const LEN: u32 = 14;
	const BYTES: u32 = 15;
	const VARS: u32 = 16;
	let empty = BlockType::Empty;
	let va = types::T_VALARRAY;
	let mut b: Vec<Instruction> = Vec::new();
	b.push(I::LocalGet(SCHEMA));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 1));
	b.push(I::LocalSet(VTAG));
	b.push(I::LocalGet(SCHEMA));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 3));
	b.push(I::LocalSet(PAYLOAD));
	let open = |b: &mut Vec<Instruction>, t: u32| {
		b.push(I::LocalGet(VTAG));
		b.push(I::I32Const(t as i32));
		b.push(I::I32Eq);
		b.push(I::If(empty));
	};
	let payload_elem = |b: &mut Vec<Instruction>, idx: i32| {
		b.push(I::LocalGet(PAYLOAD));
		b.push(I::I32Const(idx));
		b.push(I::ArrayGet(va));
	};
	// if g_err: return nothing immediately.
	let bail = |b: &mut Vec<Instruction>| {
		b.push(I::GlobalGet(g.err));
		b.push(I::If(empty));
		push_nothing(b);
		b.push(I::Return);
		b.push(I::End);
	};
	// int / duration: box(unzigzag(uvarint)).
	let int_arm = |b: &mut Vec<Instruction>, t: u32, tag: i32| {
		open(b, t);
		b.push(I::Call(ruvarint));
		b.push(I::LocalSet(U));
		bail(b);
		b.push(I::I32Const(tag));
		// (U >>u 1) ^ (0 - (U & 1)).
		b.push(I::LocalGet(U));
		b.push(I::I64Const(1));
		b.push(I::I64ShrU);
		b.push(I::I64Const(0));
		b.push(I::LocalGet(U));
		b.push(I::I64Const(1));
		b.push(I::I64And);
		b.push(I::I64Sub);
		b.push(I::I64Xor);
		b.push(I::StructNew(types::T_INT));
		b.push(I::Return);
		b.push(I::End);
	};
	int_arm(&mut b, w.s_int, types::TAG_INT);
	int_arm(&mut b, w.s_duration, types::TAG_DURATION);
	// float: 8 LE bytes → i64 → f64.
	open(&mut b, w.s_float);
	b.push(I::I64Const(0));
	b.push(I::LocalSet(BITS));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(8));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	b.push(I::LocalGet(BITS));
	b.push(I::Call(rbyte));
	b.push(I::I64ExtendI32U);
	b.push(I::I64Const(0xff));
	b.push(I::I64And);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(8));
	b.push(I::I32Mul);
	b.push(I::I64ExtendI32U);
	b.push(I::I64Shl);
	b.push(I::I64Or);
	b.push(I::LocalSet(BITS));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	bail(&mut b);
	b.push(I::I32Const(types::TAG_FLOAT));
	b.push(I::LocalGet(BITS));
	b.push(I::F64ReinterpretI64);
	b.push(I::StructNew(types::T_FLOAT));
	b.push(I::Return);
	b.push(I::End);
	// bool: one byte != 0.
	open(&mut b, w.s_bool);
	b.push(I::I32Const(types::TAG_BOOL));
	b.push(I::Call(rbyte));
	b.push(I::I32Const(0));
	b.push(I::I32Ne);
	b.push(I::StructNew(types::T_BOOL));
	b.push(I::Return);
	b.push(I::End);
	// string / bytes: uvarint length, then that many bytes. NOTE: strings are
	// taken verbatim — the VM validates UTF-8 (the `invalid-utf8` error,
	// `g_err = 3`) but we don't yet, so a non-UTF-8 wire string decodes to a
	// malformed string here rather than erroring. Unexercised by the fixtures.
	let bytes_arm = |b: &mut Vec<Instruction>, t: u32, tag: i32| {
		open(b, t);
		b.push(I::Call(ruvarint));
		b.push(I::I32WrapI64);
		b.push(I::LocalSet(LEN));
		bail(b);
		// not enough input → unexpected-end.
		b.push(I::LocalGet(LEN));
		b.push(I::GlobalGet(g.input));
		b.push(I::ArrayLen);
		b.push(I::GlobalGet(g.pos));
		b.push(I::I32Sub);
		b.push(I::I32GtU);
		b.push(I::If(empty));
		b.push(I::I32Const(1));
		b.push(I::GlobalSet(g.err));
		push_nothing(b);
		b.push(I::Return);
		b.push(I::End);
		b.push(I::LocalGet(LEN));
		b.push(I::ArrayNewDefault(types::T_BYTES));
		b.push(I::LocalSet(BYTES));
		b.push(I::I32Const(0));
		b.push(I::LocalSet(I_));
		b.push(I::Block(empty));
		b.push(I::Loop(empty));
		b.push(I::LocalGet(I_));
		b.push(I::LocalGet(LEN));
		b.push(I::I32GeU);
		b.push(I::BrIf(1));
		b.push(I::LocalGet(BYTES));
		b.push(I::LocalGet(I_));
		b.push(I::Call(rbyte));
		b.push(I::ArraySet(types::T_BYTES));
		b.push(I::LocalGet(I_));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(I_));
		b.push(I::Br(0));
		b.push(I::End); // loop
		b.push(I::End); // block
		b.push(I::I32Const(tag));
		b.push(I::LocalGet(BYTES));
		b.push(I::StructNew(types::T_STR));
		b.push(I::Return);
		b.push(I::End);
	};
	bytes_arm(&mut b, w.s_string, types::TAG_STR);
	bytes_arm(&mut b, w.s_bytes, types::TAG_BYTES);
	// nothing.
	open(&mut b, w.s_nothing);
	push_nothing(&mut b);
	b.push(I::Return);
	b.push(I::End);
	// list: uvarint count, then each element under the inner schema (payload[0]).
	open(&mut b, w.s_list);
	b.push(I::Call(ruvarint));
	b.push(I::I32WrapI64);
	b.push(I::LocalSet(N));
	bail(&mut b);
	payload_elem(&mut b, 0);
	b.push(I::LocalSet(INNER));
	b.push(I::LocalGet(N));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(OUT));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	b.push(I::LocalGet(OUT));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(INNER));
	b.push(I::Call(self_idx));
	b.push(I::ArraySet(va));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	b.push(I::I32Const(types::TAG_LIST));
	b.push(I::LocalGet(OUT));
	b.push(I::StructNew(types::T_LIST));
	b.push(I::Return);
	b.push(I::End);
	// tuple: a fixed number of fields (arity from the schema, no count on wire).
	open(&mut b, w.s_tuple);
	payload_elem(&mut b, 0);
	b.push(cast(types::T_LIST));
	b.push(getf(types::T_LIST, 1));
	b.push(I::LocalSet(SCHEMAS));
	b.push(I::LocalGet(SCHEMAS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::LocalGet(N));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(OUT));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	b.push(I::LocalGet(OUT));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(SCHEMAS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(I::Call(self_idx));
	b.push(I::ArraySet(va));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	b.push(I::I32Const(types::TAG_TUPLE));
	b.push(I::LocalGet(OUT));
	b.push(I::StructNew(types::T_TUPLE));
	b.push(I::Return);
	b.push(I::End);
	// record: decode each field in schema (name-sorted) order; build the parallel
	// names/values arrays so the `$record` is name-sorted like `MakeRecord`.
	open(&mut b, w.s_record);
	payload_elem(&mut b, 0);
	b.push(cast(types::T_LIST));
	b.push(getf(types::T_LIST, 1));
	b.push(I::LocalSet(FIELDS));
	b.push(I::LocalGet(FIELDS));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::LocalGet(N));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(NAMES));
	b.push(I::LocalGet(N));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(VALUES));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	b.push(I::LocalGet(FIELDS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_TUPLE));
	b.push(getf(types::T_TUPLE, 1));
	b.push(I::LocalSet(FT));
	// NAMES[i] = FT[0].
	b.push(I::LocalGet(NAMES));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(FT));
	b.push(I::I32Const(0));
	b.push(I::ArrayGet(va));
	b.push(I::ArraySet(va));
	// VALUES[i] = dec(FT[1]).
	b.push(I::LocalGet(VALUES));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(FT));
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(va));
	b.push(I::Call(self_idx));
	b.push(I::ArraySet(va));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	b.push(I::I32Const(types::TAG_RECORD));
	b.push(I::LocalGet(NAMES));
	b.push(I::LocalGet(VALUES));
	b.push(I::StructNew(types::T_RECORD));
	b.push(I::Return);
	b.push(I::End);
	// enum: register variants for inner `s-enum-ref`, then decode the variant.
	open(&mut b, w.s_enum);
	payload_elem(&mut b, 0);
	payload_elem(&mut b, 1);
	b.push(I::Call(ctxput));
	b.push(I::Drop);
	payload_elem(&mut b, 0);
	payload_elem(&mut b, 1);
	b.push(I::Call(dec_variant));
	b.push(I::Return);
	b.push(I::End);
	// enum-ref: resolve registered variants by name (null → malformed).
	open(&mut b, w.s_enum_ref);
	payload_elem(&mut b, 0);
	b.push(I::Call(ctxget));
	b.push(I::LocalSet(VARS));
	b.push(I::LocalGet(VARS));
	b.push(I::RefIsNull);
	b.push(I::If(empty));
	b.push(I::I32Const(5));
	b.push(I::GlobalSet(g.err));
	push_nothing(&mut b);
	b.push(I::Return);
	b.push(I::End);
	payload_elem(&mut b, 0);
	b.push(I::LocalGet(VARS));
	b.push(I::Call(dec_variant));
	b.push(I::Return);
	b.push(I::End);
	// dict: uvarint count then (key, value) pairs in wire (canonical) order. The
	// `$dict` is insertion-ordered and scans with `__eq`, so preserving decode
	// order is enough — `dict.lookup`/`size` work on the result.
	open(&mut b, w.s_dict);
	b.push(I::Call(ruvarint));
	b.push(I::I32WrapI64);
	b.push(I::LocalSet(N));
	bail(&mut b);
	b.push(I::LocalGet(N));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(OUT));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	// OUT[i] = tuple(dec(key-schema), dec(value-schema)) — key decoded first.
	b.push(I::LocalGet(OUT));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(types::TAG_TUPLE));
	payload_elem(&mut b, 0);
	b.push(I::Call(self_idx));
	payload_elem(&mut b, 1);
	b.push(I::Call(self_idx));
	b.push(I::ArrayNewFixed {
		array_type_index: va,
		array_size: 2,
	});
	b.push(I::StructNew(types::T_TUPLE));
	b.push(I::ArraySet(va));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	b.push(I::I32Const(types::TAG_DICT));
	b.push(I::LocalGet(OUT));
	b.push(I::StructNew(types::T_DICT));
	b.push(I::Return);
	b.push(I::End);
	// Fallthrough (malformed schema): nothing.
	push_nothing(&mut b);
	let locals = vec![
		ValType::I32,          // VTAG
		types::valarray_ref(), // PAYLOAD
		ValType::I32,          // N
		ValType::I32,          // I_
		types::valarray_ref(), // SCHEMAS
		types::valarray_ref(), // OUT
		types::valarray_ref(), // NAMES
		types::valarray_ref(), // VALUES
		types::valarray_ref(), // FIELDS
		types::valarray_ref(), // FT
		types::value_ref(),    // INNER
		ValType::I64,          // U
		ValType::I64,          // BITS
		ValType::I32,          // LEN
		types::bytes_ref(),    // BYTES
		types::value_ref(),    // VARS
	];
	finish(locals, &b)
}

/// Build `__wire_result(value v) -> value`: wrap a decoded value in `ok v`, or in
/// the `wire-error` variant matching `g_err` (`err …`). Runs the trailing-bytes
/// check first: a fully-decoded value with input left over is `trailing-bytes`.
pub(crate) fn build_wire_result_fn(g: WireGlobals, lits: WireResultLits) -> Function {
	use Instruction as I;
	const V: u32 = 0;
	const E: u32 = 1;
	let empty = BlockType::Empty;
	let va = types::T_VALARRAY;
	let mut b: Vec<Instruction> = Vec::new();
	// Trailing-bytes check: only when otherwise-ok and input remains.
	b.push(I::GlobalGet(g.err));
	b.push(I::I32Eqz);
	b.push(I::If(empty));
	b.push(I::GlobalGet(g.pos));
	b.push(I::GlobalGet(g.input));
	b.push(I::ArrayLen);
	b.push(I::I32LtU);
	b.push(I::If(empty));
	b.push(I::I32Const(4));
	b.push(I::GlobalSet(g.err));
	b.push(I::GlobalGet(g.input));
	b.push(I::ArrayLen);
	b.push(I::GlobalGet(g.pos));
	b.push(I::I32Sub);
	b.push(I::I64ExtendI32U);
	b.push(I::GlobalSet(g.errval));
	b.push(I::End);
	b.push(I::End);
	// ok path.
	b.push(I::GlobalGet(g.err));
	b.push(I::I32Eqz);
	b.push(I::If(empty));
	b.push(I::I32Const(types::TAG_VARIANT));
	b.push(I::I32Const(lits.ok_tag as i32));
	str_lit(&mut b, lits.ok_name);
	b.push(I::LocalGet(V));
	b.push(I::ArrayNewFixed {
		array_type_index: va,
		array_size: 1,
	});
	b.push(I::StructNew(types::T_VARIANT));
	b.push(I::Return);
	b.push(I::End);
	// err path: build the `wire-error` variant E for the error code (1..5), with
	// an `int` payload for invalid-tag / trailing-bytes.
	b.push(I::RefNull(HeapType::Concrete(types::T_VALUE)));
	b.push(I::LocalSet(E));
	for code in 1..=5i32 {
		let (etag, ename) = lits.errors[(code - 1) as usize];
		let has_payload = code == 2 || code == 4;
		b.push(I::GlobalGet(g.err));
		b.push(I::I32Const(code));
		b.push(I::I32Eq);
		b.push(I::If(empty));
		b.push(I::I32Const(types::TAG_VARIANT));
		b.push(I::I32Const(etag as i32));
		str_lit(&mut b, ename);
		if has_payload {
			b.push(I::I32Const(types::TAG_INT));
			b.push(I::GlobalGet(g.errval));
			b.push(I::StructNew(types::T_INT));
			b.push(I::ArrayNewFixed {
				array_type_index: va,
				array_size: 1,
			});
		} else {
			b.push(I::ArrayNewFixed {
				array_type_index: va,
				array_size: 0,
			});
		}
		b.push(I::StructNew(types::T_VARIANT));
		b.push(I::LocalSet(E));
		b.push(I::End);
	}
	// err(E).
	b.push(I::I32Const(types::TAG_VARIANT));
	b.push(I::I32Const(lits.err_tag as i32));
	str_lit(&mut b, lits.err_name);
	b.push(I::LocalGet(E));
	b.push(I::ArrayNewFixed {
		array_type_index: va,
		array_size: 1,
	});
	b.push(I::StructNew(types::T_VARIANT));
	finish(vec![types::value_ref()], &b)
}

/// Build `__wire_bcmp(value a, value b) -> i32`: lexicographic comparison of two
/// `$bytes`-backed values (each a `TAG_BYTES`/`$str`-shaped value), returning a
/// negative/zero/positive sign like `memcmp` with a length tie-break. Used to
/// sort dict entries by their encoded-key bytes (the canonical order).
pub(crate) fn build_wire_bcmp_fn() -> Function {
	use Instruction as I;
	const A: u32 = 0;
	const B: u32 = 1;
	const AB: u32 = 2;
	const BB: u32 = 3;
	const LA: u32 = 4;
	const LB: u32 = 5;
	const MIN: u32 = 6;
	const I_: u32 = 7;
	let empty = BlockType::Empty;
	let bytes = types::T_BYTES;
	let b: Vec<Instruction> = vec![
		I::LocalGet(A),
		cast(types::T_STR),
		getf(types::T_STR, 1),
		I::LocalSet(AB),
		I::LocalGet(B),
		cast(types::T_STR),
		getf(types::T_STR, 1),
		I::LocalSet(BB),
		I::LocalGet(AB),
		I::ArrayLen,
		I::LocalSet(LA),
		I::LocalGet(BB),
		I::ArrayLen,
		I::LocalSet(LB),
		// MIN = min(LA, LB).
		I::LocalGet(LA),
		I::LocalGet(LB),
		I::I32LtU,
		I::If(BlockType::Result(ValType::I32)),
		I::LocalGet(LA),
		I::Else,
		I::LocalGet(LB),
		I::End,
		I::LocalSet(MIN),
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(MIN),
		I::I32GeU,
		I::BrIf(1),
		// if a[i] != b[i]: return a[i] - b[i].
		I::LocalGet(AB),
		I::LocalGet(I_),
		I::ArrayGetU(bytes),
		I::LocalGet(BB),
		I::LocalGet(I_),
		I::ArrayGetU(bytes),
		I::I32Ne,
		I::If(empty),
		I::LocalGet(AB),
		I::LocalGet(I_),
		I::ArrayGetU(bytes),
		I::LocalGet(BB),
		I::LocalGet(I_),
		I::ArrayGetU(bytes),
		I::I32Sub,
		I::Return,
		I::End,
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		// common prefix equal: shorter sorts first.
		I::LocalGet(LA),
		I::LocalGet(LB),
		I::I32Sub,
	];
	finish(
		vec![
			types::bytes_ref(),
			types::bytes_ref(),
			ValType::I32,
			ValType::I32,
			ValType::I32,
			ValType::I32,
		],
		&b,
	)
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
	g: WireGlobals,
) -> Function {
	use Instruction as I;
	const SCHEMA: u32 = 0;
	const VAL: u32 = 1;
	const KSCH: u32 = 2;
	const VSCH: u32 = 3;
	const ENTRIES: u32 = 4;
	const N: u32 = 5;
	const PAIRS: u32 = 6;
	const I_: u32 = 7;
	const ENTRY: u32 = 8;
	const START: u32 = 9;
	const KEYLEN: u32 = 10;
	const KB: u32 = 11;
	const CUR: u32 = 12;
	const CURKEY: u32 = 13;
	const J: u32 = 14;
	const M: u32 = 15;
	const KL: u32 = 16;
	let empty = BlockType::Empty;
	let va = types::T_VALARRAY;
	let bytes = types::T_BYTES;
	// key/value schemas = schema.payload[0..2].
	let schema_payload = |b: &mut Vec<Instruction>, idx: i32| {
		b.push(I::LocalGet(SCHEMA));
		b.push(cast(types::T_VARIANT));
		b.push(getf(types::T_VARIANT, 3));
		b.push(I::I32Const(idx));
		b.push(I::ArrayGet(va));
	};
	// `(cast $tuple local).elems[idx]`.
	let tuple_elem = |b: &mut Vec<Instruction>, local: u32, idx: i32| {
		b.push(I::LocalGet(local));
		b.push(cast(types::T_TUPLE));
		b.push(getf(types::T_TUPLE, 1));
		b.push(I::I32Const(idx));
		b.push(I::ArrayGet(va));
	};
	let mut b: Vec<Instruction> = Vec::new();
	schema_payload(&mut b, 0);
	b.push(I::LocalSet(KSCH));
	schema_payload(&mut b, 1);
	b.push(I::LocalSet(VSCH));
	b.push(I::LocalGet(VAL));
	b.push(cast(types::T_DICT));
	b.push(getf(types::T_DICT, 1));
	b.push(I::LocalSet(ENTRIES));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::LocalGet(N));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(PAIRS));
	// Pass 1: encode each key into `g_buf`, capture its bytes, rewind. PAIRS[i] =
	// tuple(key-bytes-value, value).
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	// ENTRY = ENTRIES[i].elems.
	b.push(I::LocalGet(ENTRIES));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_TUPLE));
	b.push(getf(types::T_TUPLE, 1));
	b.push(I::LocalSet(ENTRY));
	// START = g_len; enc(KSCH, ENTRY[0]); KEYLEN = g_len - START.
	b.push(I::GlobalGet(g.len));
	b.push(I::LocalSet(START));
	b.push(I::LocalGet(KSCH));
	b.push(I::LocalGet(ENTRY));
	b.push(I::I32Const(0));
	b.push(I::ArrayGet(va));
	b.push(I::Call(enc));
	b.push(I::GlobalGet(g.len));
	b.push(I::LocalGet(START));
	b.push(I::I32Sub);
	b.push(I::LocalSet(KEYLEN));
	// KB = g_buf[START..START+KEYLEN]; rewind g_len = START.
	b.push(I::LocalGet(KEYLEN));
	b.push(I::ArrayNewDefault(bytes));
	b.push(I::LocalSet(KB));
	b.push(I::LocalGet(KB));
	b.push(I::I32Const(0));
	b.push(I::GlobalGet(g.buf));
	b.push(I::LocalGet(START));
	b.push(I::LocalGet(KEYLEN));
	b.push(I::ArrayCopy {
		array_type_index_dst: bytes,
		array_type_index_src: bytes,
	});
	b.push(I::LocalGet(START));
	b.push(I::GlobalSet(g.len));
	// PAIRS[i] = tuple( $bytes-value(KB), ENTRY[1] ).
	b.push(I::LocalGet(PAIRS));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(types::TAG_TUPLE));
	b.push(I::I32Const(types::TAG_BYTES));
	b.push(I::LocalGet(KB));
	b.push(I::StructNew(types::T_STR));
	b.push(I::LocalGet(ENTRY));
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(va));
	b.push(I::ArrayNewFixed {
		array_type_index: va,
		array_size: 2,
	});
	b.push(I::StructNew(types::T_TUPLE));
	b.push(I::ArraySet(va));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
								 // Pass 2: insertion-sort PAIRS by key bytes (stable, ascending).
	b.push(I::I32Const(1));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	b.push(I::LocalGet(PAIRS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(I::LocalSet(CUR));
	tuple_elem(&mut b, CUR, 0);
	b.push(I::LocalSet(CURKEY));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Sub);
	b.push(I::LocalSet(J));
	// while j >= 0 && bcmp(PAIRS[j].key, CURKEY) > 0: PAIRS[j+1] = PAIRS[j]; j--.
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(J));
	b.push(I::I32Const(0));
	b.push(I::I32LtS);
	b.push(I::BrIf(1));
	// key(j) = (cast $tuple PAIRS[j]).elems[0].
	b.push(I::LocalGet(PAIRS));
	b.push(I::LocalGet(J));
	b.push(I::ArrayGet(va));
	b.push(cast(types::T_TUPLE));
	b.push(getf(types::T_TUPLE, 1));
	b.push(I::I32Const(0));
	b.push(I::ArrayGet(va));
	b.push(I::LocalGet(CURKEY));
	b.push(I::Call(bcmp));
	b.push(I::I32Const(0));
	b.push(I::I32LeS);
	b.push(I::BrIf(1));
	// PAIRS[j+1] = PAIRS[j].
	b.push(I::LocalGet(PAIRS));
	b.push(I::LocalGet(J));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalGet(PAIRS));
	b.push(I::LocalGet(J));
	b.push(I::ArrayGet(va));
	b.push(I::ArraySet(va));
	b.push(I::LocalGet(J));
	b.push(I::I32Const(1));
	b.push(I::I32Sub);
	b.push(I::LocalSet(J));
	b.push(I::Br(0));
	b.push(I::End); // inner loop
	b.push(I::End); // inner block
								 // PAIRS[j+1] = CUR.
	b.push(I::LocalGet(PAIRS));
	b.push(I::LocalGet(J));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalGet(CUR));
	b.push(I::ArraySet(va));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
								 // Pass 3: uvarint(count) then each sorted (key bytes, value).
	b.push(I::LocalGet(N));
	b.push(I::I64ExtendI32U);
	b.push(I::Call(uvarint));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	// CUR = PAIRS[i]; KB = key bytes; append each byte.
	b.push(I::LocalGet(PAIRS));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(I::LocalSet(CUR));
	tuple_elem(&mut b, CUR, 0);
	b.push(cast(types::T_STR));
	b.push(getf(types::T_STR, 1));
	b.push(I::LocalSet(KB));
	b.push(I::LocalGet(KB));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(KL));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(M));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(M));
	b.push(I::LocalGet(KL));
	b.push(I::I32GeU);
	b.push(I::BrIf(1));
	b.push(I::LocalGet(KB));
	b.push(I::LocalGet(M));
	b.push(I::ArrayGetU(bytes));
	b.push(I::Call(push));
	b.push(I::LocalGet(M));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(M));
	b.push(I::Br(0));
	b.push(I::End); // byte loop
	b.push(I::End); // byte block
								 // encode value under VSCH.
	b.push(I::LocalGet(VSCH));
	tuple_elem(&mut b, CUR, 1);
	b.push(I::Call(enc));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	let locals = vec![
		types::value_ref(),    // KSCH
		types::value_ref(),    // VSCH
		types::valarray_ref(), // ENTRIES
		ValType::I32,          // N
		types::valarray_ref(), // PAIRS
		ValType::I32,          // I_
		types::valarray_ref(), // ENTRY
		ValType::I32,          // START
		ValType::I32,          // KEYLEN
		types::bytes_ref(),    // KB
		types::value_ref(),    // CUR
		types::value_ref(),    // CURKEY
		ValType::I32,          // J
		ValType::I32,          // M
		ValType::I32,          // KL
	];
	finish(locals, &b)
}
