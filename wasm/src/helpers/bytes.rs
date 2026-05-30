// Bytes helpers: tabulating builder + byte-array concat
// (`__bytes_build`, `__bytesconcat`).

use wasm_encoder::*;

use crate::types;

/// Build `__bytes_build(n, f) -> bytes`: tabulate an `n`-byte sequence, calling
/// `f` per index and storing its int result (truncated to 8 bits by the packed
/// `$bytes` array). `arity1` is the 1-arg closure func-type index.
pub(crate) fn build_bytes_build_fn(arity1: u32) -> Function {
	use Instruction as I;
	const N: u32 = 0; // param: n (boxed int)
	const F: u32 = 1; // param: f (closure)
	const NLEN: u32 = 2;
	const BUF: u32 = 3;
	const I_: u32 = 4;
	let empty = wasm_encoder::BlockType::Empty;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let locals = vec![ValType::I32, types::bytes_ref(), ValType::I32];
	let b: Vec<Instruction> = vec![
		I::LocalGet(N),
		cast(types::T_INT),
		getf(types::T_INT, 1),
		I::I32WrapI64,
		I::LocalSet(NLEN),
		I::LocalGet(NLEN),
		I::ArrayNewDefault(types::T_BYTES),
		I::LocalSet(BUF),
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(NLEN),
		I::I32GeS,
		I::BrIf(1),
		// buf[i] = (i32) f(box i)
		I::LocalGet(BUF),
		I::LocalGet(I_),
		I::LocalGet(F),
		cast(types::T_CLOSURE), // env
		I::I32Const(types::TAG_INT),
		I::LocalGet(I_),
		I::I64ExtendI32S,
		I::StructNew(types::T_INT), // arg = box i
		I::LocalGet(F),
		cast(types::T_CLOSURE),
		getf(types::T_CLOSURE, 1), // fn_index
		I::CallIndirect {
			type_index: arity1,
			table_index: 0,
		},
		cast(types::T_INT),
		getf(types::T_INT, 1),
		I::I32WrapI64, // unbox result to i32 (array.set packs to i8)
		I::ArraySet(types::T_BYTES),
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::I32Const(types::TAG_BYTES),
		I::LocalGet(BUF),
		I::StructNew(types::T_STR),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__bytesconcat(a, b) -> bytes`: a fresh byte array holding `a` then `b`.
pub(crate) fn build_bytesconcat_fn() -> Function {
	use Instruction as I;
	const A: u32 = 0;
	const B: u32 = 1;
	const LA: u32 = 2;
	const LB: u32 = 3;
	const DST: u32 = 4;
	let bv = types::T_BYTES;
	let copy = I::ArrayCopy {
		array_type_index_dst: bv,
		array_type_index_src: bv,
	};
	let locals = vec![ValType::I32, ValType::I32, types::bytes_ref()];
	let b: Vec<Instruction> = vec![
		I::LocalGet(A),
		I::ArrayLen,
		I::LocalSet(LA),
		I::LocalGet(B),
		I::ArrayLen,
		I::LocalSet(LB),
		I::LocalGet(LA),
		I::LocalGet(LB),
		I::I32Add,
		I::ArrayNewDefault(bv),
		I::LocalSet(DST),
		I::LocalGet(DST),
		I::I32Const(0),
		I::LocalGet(A),
		I::I32Const(0),
		I::LocalGet(LA),
		copy.clone(),
		I::LocalGet(DST),
		I::LocalGet(LA),
		I::LocalGet(B),
		I::I32Const(0),
		I::LocalGet(LB),
		copy,
		I::LocalGet(DST),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}
