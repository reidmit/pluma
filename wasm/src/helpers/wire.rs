// `wire` codec helpers: the FNV-1a mixers and the recursive schema fingerprint
// (`__wire_mix_len`, `__wire_mix_str`, `__wire_fp`).

use wasm_encoder::*;

use crate::runtime::{WireTags, WIRE_FNV_PRIME};
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
