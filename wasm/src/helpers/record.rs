// Record field access + update (`__getfield`, `__record_update`).

use wasm_encoder::*;

use crate::types;

/// Build `__getfield(record, name) -> value`: linear-scan the record's
/// name-sorted `names` array, comparing each to `name` via `__eq`; return the
/// parallel `values` element on match. Traps if absent (the type checker
/// guarantees the field exists).
pub(crate) fn build_getfield_fn(eq_idx: u32) -> Function {
	use Instruction as I;
	const REC: u32 = 0;
	const NAME: u32 = 1;
	const NAMES: u32 = 2;
	const VALUES: u32 = 3;
	const N: u32 = 4;
	const I_: u32 = 5;
	let empty = wasm_encoder::BlockType::Empty;
	let locals = vec![
		types::valarray_ref(),
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
	];
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let mut b: Vec<Instruction> = vec![
		I::LocalGet(REC),
		cast(types::T_RECORD),
		getf(types::T_RECORD, 1),
		I::LocalSet(NAMES),
		I::LocalGet(REC),
		cast(types::T_RECORD),
		getf(types::T_RECORD, 2),
		I::LocalSet(VALUES),
		I::LocalGet(NAMES),
		I::ArrayLen,
		I::LocalSet(N),
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(N),
		I::I32GeS,
		I::BrIf(1), // not found -> fall out (then trap)
		I::LocalGet(NAMES),
		I::LocalGet(I_),
		I::ArrayGet(types::T_VALARRAY),
		I::LocalGet(NAME),
		I::Call(eq_idx),
		I::If(empty),
		I::LocalGet(VALUES),
		I::LocalGet(I_),
		I::ArrayGet(types::T_VALARRAY),
		I::Return,
		I::End,
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0),
		I::End, // loop
		I::End, // block
		I::Unreachable,
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in b.drain(..) {
		f.instruction(&ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__record_update(rec, name, value) -> rec`: a copy of `rec` with the
/// field named `name` overridden. Shares `rec`'s name array; copies its values
/// and replaces the matching slot (found via `__eq` on names).
pub(crate) fn build_record_update_fn(eq_idx: u32) -> Function {
	use Instruction as I;
	const REC: u32 = 0;
	const NAME: u32 = 1;
	const VALUE: u32 = 2;
	const NAMES: u32 = 3;
	const VALUES: u32 = 4;
	const NEW: u32 = 5;
	const N: u32 = 6;
	const I_: u32 = 7;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let locals = vec![
		types::valarray_ref(),
		types::valarray_ref(),
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
	];
	let b: Vec<Instruction> = vec![
		I::LocalGet(REC),
		cast(types::T_RECORD),
		getf(types::T_RECORD, 1),
		I::LocalSet(NAMES),
		I::LocalGet(REC),
		cast(types::T_RECORD),
		getf(types::T_RECORD, 2),
		I::LocalSet(VALUES),
		I::LocalGet(VALUES),
		I::ArrayLen,
		I::LocalSet(N),
		// new = copy of values
		I::LocalGet(N),
		I::ArrayNewDefault(va),
		I::LocalSet(NEW),
		I::LocalGet(NEW),
		I::I32Const(0),
		I::LocalGet(VALUES),
		I::I32Const(0),
		I::LocalGet(N),
		I::ArrayCopy {
			array_type_index_dst: va,
			array_type_index_src: va,
		},
		// find name; new[i] = value; stop
		I::I32Const(0),
		I::LocalSet(I_),
		I::Block(empty),
		I::Loop(empty),
		I::LocalGet(I_),
		I::LocalGet(N),
		I::I32GeS,
		I::BrIf(1), // not found -> done
		I::LocalGet(NAMES),
		I::LocalGet(I_),
		I::ArrayGet(va),
		I::LocalGet(NAME),
		I::Call(eq_idx),
		I::If(empty),
		I::LocalGet(NEW),
		I::LocalGet(I_),
		I::LocalGet(VALUE),
		I::ArraySet(va),
		I::Br(2), // -> done
		I::End,
		I::LocalGet(I_),
		I::I32Const(1),
		I::I32Add,
		I::LocalSet(I_),
		I::Br(0), // -> loop
		I::End,   // loop
		I::End,   // block
		I::I32Const(types::TAG_RECORD),
		I::LocalGet(NAMES),
		I::LocalGet(NEW),
		I::StructNew(types::T_RECORD),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}
