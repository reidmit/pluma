// Structural equality (`__eq`).

use wasm_encoder::*;

use crate::types;

/// Build the structural-equality runtime helper `__eq(a, b) -> i32` (1 = equal).
/// Recursive over variants; loops over string bytes. Mirrors `vm`'s structural
/// `==`: same-typed operands (the type checker guarantees it), IEEE float compare
/// (so `nan != nan`), byte-exact strings. `self_idx` is `__eq`'s own wasm index
/// (for the variant-payload recursion). Tuples/lists/records are not yet handled
/// (they trap — a clear signal to implement them, not a silent wrong answer).
pub(crate) fn build_eq_fn(self_idx: u32) -> Function {
	use Instruction as I;
	// Locals past the two params: ta, tb, i, n (i32); aa, bb ($bytes); pa, pb
	// ($valarray); j, found (i32, for the order-independent dict compare).
	let locals = vec![
		ValType::I32,
		ValType::I32,
		ValType::I32,
		ValType::I32,
		types::bytes_ref(),
		types::bytes_ref(),
		types::valarray_ref(),
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
	];
	const A: u32 = 0;
	const B: u32 = 1;
	const TA: u32 = 2;
	const TB: u32 = 3;
	const I_: u32 = 4;
	const N: u32 = 5;
	const AA: u32 = 6;
	const BB: u32 = 7;
	const PA: u32 = 8;
	const PB: u32 = 9;
	const J: u32 = 10;
	const FOUND: u32 = 11;
	let empty = wasm_encoder::BlockType::Empty;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	let mut b: Vec<Instruction> = Vec::new();
	// ta = tag(a); tb = tag(b); if ta != tb -> 0.
	b.push(I::LocalGet(A));
	b.push(getf(types::T_VALUE, 0));
	b.push(I::LocalSet(TA));
	b.push(I::LocalGet(B));
	b.push(getf(types::T_VALUE, 0));
	b.push(I::LocalSet(TB));
	b.push(I::LocalGet(TA));
	b.push(I::LocalGet(TB));
	b.push(I::I32Ne);
	b.push(I::If(empty));
	b.push(I::I32Const(0));
	b.push(I::Return);
	b.push(I::End);
	// Per-tag scalar cases, each returning.
	let scalar = |b: &mut Vec<Instruction>, tag: i32, ty: u32, eq: Instruction<'static>| {
		b.push(I::LocalGet(TA));
		b.push(I::I32Const(tag));
		b.push(I::I32Eq);
		b.push(I::If(empty));
		b.push(I::LocalGet(A));
		b.push(cast(ty));
		b.push(getf(ty, 1));
		b.push(I::LocalGet(B));
		b.push(cast(ty));
		b.push(getf(ty, 1));
		b.push(eq);
		b.push(I::Return);
		b.push(I::End);
	};
	// NOTHING -> equal.
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_NOTHING));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	b.push(I::I32Const(1));
	b.push(I::Return);
	b.push(I::End);
	scalar(&mut b, types::TAG_BOOL, types::T_BOOL, I::I32Eq);
	scalar(&mut b, types::TAG_INT, types::T_INT, I::I64Eq);
	scalar(&mut b, types::TAG_FLOAT, types::T_FLOAT, I::F64Eq);
	// STR / BYTES (same `{tag, $bytes}` shape): equal lengths and equal bytes.
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_STR));
	b.push(I::I32Eq);
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_BYTES));
	b.push(I::I32Eq);
	b.push(I::I32Or);
	b.push(I::If(empty));
	{
		b.push(I::LocalGet(A));
		b.push(cast(types::T_STR));
		b.push(getf(types::T_STR, 1));
		b.push(I::LocalSet(AA));
		b.push(I::LocalGet(B));
		b.push(cast(types::T_STR));
		b.push(getf(types::T_STR, 1));
		b.push(I::LocalSet(BB));
		b.push(I::LocalGet(AA));
		b.push(I::ArrayLen);
		b.push(I::LocalSet(N));
		b.push(I::LocalGet(BB));
		b.push(I::ArrayLen);
		b.push(I::LocalGet(N));
		b.push(I::I32Ne);
		b.push(I::If(empty));
		b.push(I::I32Const(0));
		b.push(I::Return);
		b.push(I::End);
		b.push(I::I32Const(0));
		b.push(I::LocalSet(I_));
		b.push(I::Block(empty)); // $brk
		b.push(I::Loop(empty)); // $lp
		b.push(I::LocalGet(I_));
		b.push(I::LocalGet(N));
		b.push(I::I32GeS);
		b.push(I::BrIf(1)); // -> $brk
		b.push(I::LocalGet(AA));
		b.push(I::LocalGet(I_));
		b.push(I::ArrayGetU(types::T_BYTES));
		b.push(I::LocalGet(BB));
		b.push(I::LocalGet(I_));
		b.push(I::ArrayGetU(types::T_BYTES));
		b.push(I::I32Ne);
		b.push(I::If(empty));
		b.push(I::I32Const(0));
		b.push(I::Return);
		b.push(I::End);
		b.push(I::LocalGet(I_));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(I_));
		b.push(I::Br(0)); // -> $lp
		b.push(I::End); // loop
		b.push(I::End); // block
		b.push(I::I32Const(1));
		b.push(I::Return);
	}
	b.push(I::End);
	// Element-wise array compare (recursive). Loads the `$valarray` at field
	// `field` of both `a`/`b` (cast to `sty`), checks equal lengths, then compares
	// each element via `__eq`; emits the success `return 1`.
	let cmp_array = |b: &mut Vec<Instruction>, sty: u32, field: u32| {
		b.push(I::LocalGet(A));
		b.push(cast(sty));
		b.push(getf(sty, field));
		b.push(I::LocalSet(PA));
		b.push(I::LocalGet(B));
		b.push(cast(sty));
		b.push(getf(sty, field));
		b.push(I::LocalSet(PB));
		// Lengths must match.
		b.push(I::LocalGet(PA));
		b.push(I::ArrayLen);
		b.push(I::LocalSet(N));
		b.push(I::LocalGet(PB));
		b.push(I::ArrayLen);
		b.push(I::LocalGet(N));
		b.push(I::I32Ne);
		b.push(I::If(empty));
		b.push(I::I32Const(0));
		b.push(I::Return);
		b.push(I::End);
		b.push(I::I32Const(0));
		b.push(I::LocalSet(I_));
		b.push(I::Block(empty)); // $brk
		b.push(I::Loop(empty)); // $lp
		b.push(I::LocalGet(I_));
		b.push(I::LocalGet(N));
		b.push(I::I32GeS);
		b.push(I::BrIf(1)); // -> $brk
		b.push(I::LocalGet(PA));
		b.push(I::LocalGet(I_));
		b.push(I::ArrayGet(types::T_VALARRAY));
		b.push(I::LocalGet(PB));
		b.push(I::LocalGet(I_));
		b.push(I::ArrayGet(types::T_VALARRAY));
		b.push(I::Call(self_idx));
		b.push(I::I32Eqz);
		b.push(I::If(empty));
		b.push(I::I32Const(0));
		b.push(I::Return);
		b.push(I::End);
		b.push(I::LocalGet(I_));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(I_));
		b.push(I::Br(0)); // -> $lp
		b.push(I::End); // loop
		b.push(I::End); // block
		b.push(I::I32Const(1));
		b.push(I::Return);
	};
	// VARIANT: equal tags, then equal payloads.
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_VARIANT));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	b.push(I::LocalGet(A));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 1));
	b.push(I::LocalGet(B));
	b.push(cast(types::T_VARIANT));
	b.push(getf(types::T_VARIANT, 1));
	b.push(I::I32Ne);
	b.push(I::If(empty));
	b.push(I::I32Const(0));
	b.push(I::Return);
	b.push(I::End);
	cmp_array(&mut b, types::T_VARIANT, 3);
	b.push(I::End);
	// TUPLE / LIST: compare the element arrays. RECORD: compare the values arrays
	// (same type ⇒ same name-sorted fields, so positional value compare suffices).
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_TUPLE));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	cmp_array(&mut b, types::T_TUPLE, 1);
	b.push(I::End);
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_LIST));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	cmp_array(&mut b, types::T_LIST, 1);
	b.push(I::End);
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_RECORD));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	cmp_array(&mut b, types::T_RECORD, 2);
	b.push(I::End);
	// REF: reference identity (`ref.eq`), matching the VM's `Rc::ptr_eq` — two
	// cells are equal iff they are the same cell, regardless of contents.
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_REF));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	b.push(I::LocalGet(A));
	b.push(I::LocalGet(B));
	b.push(I::RefEq);
	b.push(I::Return);
	b.push(I::End);
	// DICT: order-independent structural compare (matches the VM). Equal sizes,
	// then every entry of `a` must have a key in `b` with an equal value. Keys are
	// unique within each dict, so equal sizes make this a bijection check. Entry
	// fields are read inline: `entries[idx]` is a `$tuple`, elem 0 = key, 1 = value.
	let entry_field = |b: &mut Vec<Instruction>, arr: u32, idx: u32, field: i32| {
		b.push(I::LocalGet(arr));
		b.push(I::LocalGet(idx));
		b.push(I::ArrayGet(types::T_VALARRAY));
		b.push(cast(types::T_TUPLE));
		b.push(getf(types::T_TUPLE, 1));
		b.push(I::I32Const(field));
		b.push(I::ArrayGet(types::T_VALARRAY));
	};
	b.push(I::LocalGet(TA));
	b.push(I::I32Const(types::TAG_DICT));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	// PA = a.entries; PB = b.entries; N = len(a); bail if lengths differ.
	b.push(I::LocalGet(A));
	b.push(cast(types::T_DICT));
	b.push(getf(types::T_DICT, 1));
	b.push(I::LocalSet(PA));
	b.push(I::LocalGet(B));
	b.push(cast(types::T_DICT));
	b.push(getf(types::T_DICT, 1));
	b.push(I::LocalSet(PB));
	b.push(I::LocalGet(PA));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::LocalGet(PB));
	b.push(I::ArrayLen);
	b.push(I::LocalGet(N));
	b.push(I::I32Ne);
	b.push(I::If(empty));
	b.push(I::I32Const(0));
	b.push(I::Return);
	b.push(I::End);
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty)); // $outer
	b.push(I::Loop(empty)); // $oloop
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1)); // -> $outer (done; all matched)
	b.push(I::I32Const(0));
	b.push(I::LocalSet(J));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(FOUND));
	b.push(I::Block(empty)); // $inner
	b.push(I::Loop(empty)); // $iloop
	b.push(I::LocalGet(J));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1)); // -> $inner (key absent in b)
										 // if __eq(a.key[i], b.key[j]) { ... }
	entry_field(&mut b, PA, I_, 0);
	entry_field(&mut b, PB, J, 0);
	b.push(I::Call(self_idx));
	b.push(I::If(empty));
	// values must match, else the dicts differ.
	entry_field(&mut b, PA, I_, 1);
	entry_field(&mut b, PB, J, 1);
	b.push(I::Call(self_idx));
	b.push(I::I32Eqz);
	b.push(I::If(empty));
	b.push(I::I32Const(0));
	b.push(I::Return);
	b.push(I::End);
	b.push(I::I32Const(1));
	b.push(I::LocalSet(FOUND));
	b.push(I::Br(2)); // -> $inner (key found, move to next a-entry)
	b.push(I::End); // if key-eq
	b.push(I::LocalGet(J));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(J));
	b.push(I::Br(0)); // -> $iloop
	b.push(I::End); // $iloop
	b.push(I::End); // $inner
								 // a-key absent in b -> not equal.
	b.push(I::LocalGet(FOUND));
	b.push(I::I32Eqz);
	b.push(I::If(empty));
	b.push(I::I32Const(0));
	b.push(I::Return);
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0)); // -> $oloop
	b.push(I::End); // $oloop
	b.push(I::End); // $outer
	b.push(I::I32Const(1));
	b.push(I::Return);
	b.push(I::End);
	// Unhandled (closure/ctor): not structurally compared.
	b.push(I::Unreachable);
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}
