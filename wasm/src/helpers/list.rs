// List helpers: `...rest` tail, tabulating builders, and value-array concat
// (`__list_tail`, `__list_build`, `__list_collect`, `__arrconcat`).

use wasm_encoder::*;

use crate::types;

/// Build `__list_tail(list, n) -> list`: a fresh list of the elements from index
/// `n` (the `...rest` of a list pattern). `n` is a boxed int.
pub(crate) fn build_list_tail_fn() -> Function {
	use Instruction as I;
	const LIST: u32 = 0;
	const NARG: u32 = 1;
	const SRC: u32 = 2;
	const DST: u32 = 3;
	const LEN: u32 = 4;
	const N: u32 = 5;
	const I_: u32 = 6;
	let empty = wasm_encoder::BlockType::Empty;
	let locals = vec![
		types::valarray_ref(),
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
		ValType::I32,
	];
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let mut b: Vec<Instruction> = vec![
		I::LocalGet(LIST),
		cast(types::T_LIST),
		getf(types::T_LIST, 1),
		I::LocalSet(SRC),
		I::LocalGet(SRC),
		I::ArrayLen,
		I::LocalSet(LEN),
		// n = (int) NARG
		I::LocalGet(NARG),
		cast(types::T_INT),
		getf(types::T_INT, 1),
		I::I32WrapI64,
		I::LocalSet(N),
		// dst = new valarray of (len - n)
		I::LocalGet(LEN),
		I::LocalGet(N),
		I::I32Sub,
		I::ArrayNewDefault(types::T_VALARRAY),
		I::LocalSet(DST),
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		// i >= len - n -> done
		I::LocalGet(I_),
		I::LocalGet(LEN),
		I::LocalGet(N),
		I::I32Sub,
		I::I32GeS,
		I::BrIf(1),
		// dst[i] = src[n + i]
		I::LocalGet(DST),
		I::LocalGet(I_),
		I::LocalGet(SRC),
		I::LocalGet(N),
		I::LocalGet(I_),
		I::I32Add,
		I::ArrayGet(types::T_VALARRAY),
		I::ArraySet(types::T_VALARRAY),
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::I32Const(types::TAG_LIST),
		I::LocalGet(DST),
		I::StructNew(types::T_LIST),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in b.drain(..) {
		f.instruction(&ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__list_build(n, f) -> list`: tabulate `[f 0, f 1, ..., f (n-1)]` in
/// one pass. `arity1` is the wasm func-type index for a 1-arg closure (env-first
/// `(value, value) -> value`), used to `call_indirect` through `f`.
pub(crate) fn build_list_build_fn(arity1: u32) -> Function {
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
	let locals = vec![ValType::I32, types::valarray_ref(), ValType::I32];
	let b: Vec<Instruction> = vec![
		// nlen = (int) n
		I::LocalGet(N),
		cast(types::T_INT),
		getf(types::T_INT, 1),
		I::I32WrapI64,
		I::LocalSet(NLEN),
		// buf = new valarray(nlen)
		I::LocalGet(NLEN),
		I::ArrayNewDefault(types::T_VALARRAY),
		I::LocalSet(BUF),
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(NLEN),
		I::I32GeS,
		I::BrIf(1),
		// buf[i] = f(box i)
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
		I::ArraySet(types::T_VALARRAY),
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::I32Const(types::TAG_LIST),
		I::LocalGet(BUF),
		I::StructNew(types::T_LIST),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__list_collect(n, f) -> list`: like `__list_build`, but `f` returns an
/// `option`; keep each `some`'s payload in order (detected by a non-empty variant
/// payload), then trim the over-allocated buffer to the kept count.
pub(crate) fn build_list_collect_fn(arity1: u32) -> Function {
	use Instruction as I;
	const N: u32 = 0; // param: n (boxed int)
	const F: u32 = 1; // param: f (closure)
	const NLEN: u32 = 2;
	const BUF: u32 = 3;
	const I_: u32 = 4;
	const W: u32 = 5; // write cursor (kept count)
	const R: u32 = 6; // f's result (an option variant)
	const OUT: u32 = 7;
	let empty = wasm_encoder::BlockType::Empty;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let locals = vec![
		ValType::I32,          // NLEN
		types::valarray_ref(), // BUF
		ValType::I32,          // I_
		ValType::I32,          // W
		types::value_ref(),    // R
		types::valarray_ref(), // OUT
	];
	let b: Vec<Instruction> = vec![
		I::LocalGet(N),
		cast(types::T_INT),
		getf(types::T_INT, 1),
		I::I32WrapI64,
		I::LocalSet(NLEN),
		I::LocalGet(NLEN),
		I::ArrayNewDefault(types::T_VALARRAY),
		I::LocalSet(BUF),
		I::I32Const(0),
		I::LocalSet(I_),
		I::I32Const(0),
		I::LocalSet(W),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(NLEN),
		I::I32GeS,
		I::BrIf(1),
		// r = f(box i)
		I::LocalGet(F),
		cast(types::T_CLOSURE),
		I::I32Const(types::TAG_INT),
		I::LocalGet(I_),
		I::I64ExtendI32S,
		I::StructNew(types::T_INT),
		I::LocalGet(F),
		cast(types::T_CLOSURE),
		getf(types::T_CLOSURE, 1),
		I::CallIndirect {
			type_index: arity1,
			table_index: 0,
		},
		I::LocalSet(R),
		// if r's payload is non-empty (some): buf[w] = payload[0]; w += 1
		I::LocalGet(R),
		cast(types::T_VARIANT),
		getf(types::T_VARIANT, 3),
		I::ArrayLen,
		I::If(empty),
		I::LocalGet(BUF),
		I::LocalGet(W),
		I::LocalGet(R),
		cast(types::T_VARIANT),
		getf(types::T_VARIANT, 3),
		I::I32Const(0),
		I::ArrayGet(types::T_VALARRAY),
		I::ArraySet(types::T_VALARRAY),
		I::LocalGet(W),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(W),
		I::End, // if
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		// out = new valarray(w); out[0..w] = buf[0..w]
		I::LocalGet(W),
		I::ArrayNewDefault(types::T_VALARRAY),
		I::LocalSet(OUT),
		I::LocalGet(OUT),
		I::I32Const(0),
		I::LocalGet(BUF),
		I::I32Const(0),
		I::LocalGet(W),
		I::ArrayCopy {
			array_type_index_dst: types::T_VALARRAY,
			array_type_index_src: types::T_VALARRAY,
		},
		I::I32Const(types::TAG_LIST),
		I::LocalGet(OUT),
		I::StructNew(types::T_LIST),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__arrconcat(a, b) -> valarray`: a fresh array holding `a` then `b`.
pub(crate) fn build_arrconcat_fn() -> Function {
	use Instruction as I;
	const A: u32 = 0;
	const B: u32 = 1;
	const LA: u32 = 2;
	const LB: u32 = 3;
	const DST: u32 = 4;
	let va = types::T_VALARRAY;
	let copy = I::ArrayCopy {
		array_type_index_dst: va,
		array_type_index_src: va,
	};
	let locals = vec![ValType::I32, ValType::I32, types::valarray_ref()];
	let b: Vec<Instruction> = vec![
		I::LocalGet(A),
		I::ArrayLen,
		I::LocalSet(LA),
		I::LocalGet(B),
		I::ArrayLen,
		I::LocalSet(LB),
		// dst = new valarray(la + lb)
		I::LocalGet(LA),
		I::LocalGet(LB),
		I::I32Add,
		I::ArrayNewDefault(va),
		I::LocalSet(DST),
		// dst[0..la] = a
		I::LocalGet(DST),
		I::I32Const(0),
		I::LocalGet(A),
		I::I32Const(0),
		I::LocalGet(LA),
		copy.clone(),
		// dst[la..la+lb] = b
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
