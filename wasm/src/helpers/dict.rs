// Dict helpers: insert/lookup/remove/map/filter over the insertion-ordered
// `$dict` entries array (linear scan via `__eq`).

use wasm_encoder::*;

use crate::runtime::OptionLits;
use crate::types;

// ---------------------------------------------------------------------------
// Dict helpers. A `$dict` is `{tag, $valarray entries}` where each entry is a
// `$tuple (key, value)`. We linear-scan with `__eq` on keys — the VM's hash
// buckets are a pure accelerator, so insertion-order + structural key equality
// fully determine observable behavior. insert/lookup/remove DROP the hash
// method-dict the `where (hash k)` constraint passes (handled at the call site).
// ---------------------------------------------------------------------------

/// Emit the `$valarray` of the dict in `D` (param 0), i.e. `D.entries`.
fn dict_entries_of(b: &mut Vec<Instruction>, d: u32) {
	use Instruction as I;
	b.push(I::LocalGet(d));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_DICT)));
	b.push(I::StructGet {
		struct_type_index: types::T_DICT,
		field_index: 1,
	});
}

/// Emit `entries[idx_local].elems[field]` — the key (field 0) or value (1) of
/// the `$tuple` entry at `idx_local` in the `$valarray` held in `arr_local`.
fn dict_entry_field(b: &mut Vec<Instruction>, arr_local: u32, idx_local: u32, field: i32) {
	use Instruction as I;
	b.push(I::LocalGet(arr_local));
	b.push(I::LocalGet(idx_local));
	b.push(I::ArrayGet(types::T_VALARRAY));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_TUPLE)));
	b.push(I::StructGet {
		struct_type_index: types::T_TUPLE,
		field_index: 1,
	});
	b.push(I::I32Const(field));
	b.push(I::ArrayGet(types::T_VALARRAY));
}

/// Build `__dict_insert(dict, key, value) -> dict`: scan for `key` (via `__eq`);
/// replace its entry if present, else append. Returns a fresh `$dict`.
pub(crate) fn build_dict_insert_fn(eq_idx: u32) -> Function {
	use Instruction as I;
	const D: u32 = 0;
	const K: u32 = 1;
	const V: u32 = 2;
	const ENTRIES: u32 = 3;
	const N: u32 = 4;
	const I_: u32 = 5;
	const FOUND: u32 = 6;
	const NEW: u32 = 7;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	// `NEW[at] = tuple(K, V)`.
	let store_kv = |b: &mut Vec<Instruction>, at: &dyn Fn(&mut Vec<Instruction>)| {
		b.push(I::LocalGet(NEW));
		at(b);
		b.push(I::I32Const(types::TAG_TUPLE));
		b.push(I::LocalGet(K));
		b.push(I::LocalGet(V));
		b.push(I::ArrayNewFixed {
			array_type_index: va,
			array_size: 2,
		});
		b.push(I::StructNew(types::T_TUPLE));
		b.push(I::ArraySet(va));
	};
	let mut b: Vec<Instruction> = Vec::new();
	dict_entries_of(&mut b, D);
	b.push(I::LocalSet(ENTRIES));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(-1));
	b.push(I::LocalSet(FOUND));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	// Keys are unique, so the last (==only) match is the entry to replace.
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	dict_entry_field(&mut b, ENTRIES, I_, 0);
	b.push(I::LocalGet(K));
	b.push(I::Call(eq_idx));
	b.push(I::If(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalSet(FOUND));
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
								 // Pre-init NEW (a non-null local) so it is definitely-assigned on every path
								 // — the validator does not merge assignments made only inside if/else arms.
	b.push(I::LocalGet(ENTRIES));
	b.push(I::LocalSet(NEW));
	b.push(I::LocalGet(FOUND));
	b.push(I::I32Const(0));
	b.push(I::I32GeS);
	b.push(I::If(empty));
	{
		// Replace: NEW = copy of ENTRIES; NEW[FOUND] = (K, V).
		b.push(I::LocalGet(N));
		b.push(I::ArrayNewDefault(va));
		b.push(I::LocalSet(NEW));
		b.push(I::LocalGet(NEW));
		b.push(I::I32Const(0));
		b.push(I::LocalGet(ENTRIES));
		b.push(I::I32Const(0));
		b.push(I::LocalGet(N));
		b.push(I::ArrayCopy {
			array_type_index_dst: va,
			array_type_index_src: va,
		});
		store_kv(&mut b, &|b| b.push(I::LocalGet(FOUND)));
	}
	b.push(I::Else);
	{
		// Append: NEW = copy of ENTRIES grown by one; NEW[N] = (K, V).
		b.push(I::LocalGet(N));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::ArrayNewDefault(va));
		b.push(I::LocalSet(NEW));
		b.push(I::LocalGet(NEW));
		b.push(I::I32Const(0));
		b.push(I::LocalGet(ENTRIES));
		b.push(I::I32Const(0));
		b.push(I::LocalGet(N));
		b.push(I::ArrayCopy {
			array_type_index_dst: va,
			array_type_index_src: va,
		});
		store_kv(&mut b, &|b| b.push(I::LocalGet(N)));
	}
	b.push(I::End);
	b.push(I::I32Const(types::TAG_DICT));
	b.push(I::LocalGet(NEW));
	b.push(I::StructNew(types::T_DICT));
	let locals = vec![
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
		ValType::I32,
		types::valarray_ref(),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__dict_lookup(dict, key) -> option value`: linear scan via `__eq`.
pub(crate) fn build_dict_lookup_fn(eq_idx: u32, opt: OptionLits) -> Function {
	use Instruction as I;
	const D: u32 = 0;
	const K: u32 = 1;
	const ENTRIES: u32 = 2;
	const N: u32 = 3;
	const I_: u32 = 4;
	let empty = wasm_encoder::BlockType::Empty;
	// Push a fresh `$str` for an interned data-segment literal.
	let str_lit = |b: &mut Vec<Instruction>, (off, len): (u32, u32)| {
		b.push(I::I32Const(types::TAG_STR));
		b.push(I::I32Const(off as i32));
		b.push(I::I32Const(len as i32));
		b.push(I::ArrayNewData {
			array_type_index: types::T_BYTES,
			array_data_index: 0,
		});
		b.push(I::StructNew(types::T_STR));
	};
	let mut b: Vec<Instruction> = Vec::new();
	dict_entries_of(&mut b, D);
	b.push(I::LocalSet(ENTRIES));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	dict_entry_field(&mut b, ENTRIES, I_, 0);
	b.push(I::LocalGet(K));
	b.push(I::Call(eq_idx));
	b.push(I::If(empty));
	// return some(value).
	b.push(I::I32Const(types::TAG_VARIANT));
	b.push(I::I32Const(opt.some_tag as i32));
	str_lit(&mut b, opt.some_name);
	dict_entry_field(&mut b, ENTRIES, I_, 1);
	b.push(I::ArrayNewFixed {
		array_type_index: types::T_VALARRAY,
		array_size: 1,
	});
	b.push(I::StructNew(types::T_VARIANT));
	b.push(I::Return);
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
								 // none.
	b.push(I::I32Const(types::TAG_VARIANT));
	b.push(I::I32Const(opt.none_tag as i32));
	str_lit(&mut b, opt.none_name);
	b.push(I::ArrayNewFixed {
		array_type_index: types::T_VALARRAY,
		array_size: 0,
	});
	b.push(I::StructNew(types::T_VARIANT));
	let locals = vec![types::valarray_ref(), ValType::I32, ValType::I32];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__dict_remove(dict, key) -> dict`: drop the matching entry (renumbered
/// dense). Returns the original dict unchanged when the key is absent.
pub(crate) fn build_dict_remove_fn(eq_idx: u32) -> Function {
	use Instruction as I;
	const D: u32 = 0;
	const K: u32 = 1;
	const ENTRIES: u32 = 2;
	const N: u32 = 3;
	const I_: u32 = 4;
	const FOUND: u32 = 5;
	const NEW: u32 = 6;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	let mut b: Vec<Instruction> = Vec::new();
	dict_entries_of(&mut b, D);
	b.push(I::LocalSet(ENTRIES));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(-1));
	b.push(I::LocalSet(FOUND));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	dict_entry_field(&mut b, ENTRIES, I_, 0);
	b.push(I::LocalGet(K));
	b.push(I::Call(eq_idx));
	b.push(I::If(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalSet(FOUND));
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
								 // Absent: hand back the original dict.
	b.push(I::LocalGet(FOUND));
	b.push(I::I32Const(0));
	b.push(I::I32LtS);
	b.push(I::If(empty));
	b.push(I::LocalGet(D));
	b.push(I::Return);
	b.push(I::End);
	// NEW = array(N-1); copy [0..FOUND) then (FOUND..N) shifted down by one.
	b.push(I::LocalGet(N));
	b.push(I::I32Const(1));
	b.push(I::I32Sub);
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(NEW));
	b.push(I::LocalGet(NEW));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(FOUND));
	b.push(I::ArrayCopy {
		array_type_index_dst: va,
		array_type_index_src: va,
	});
	b.push(I::LocalGet(NEW));
	b.push(I::LocalGet(FOUND));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::LocalGet(FOUND));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalGet(N));
	b.push(I::I32Const(1));
	b.push(I::I32Sub);
	b.push(I::LocalGet(FOUND));
	b.push(I::I32Sub);
	b.push(I::ArrayCopy {
		array_type_index_dst: va,
		array_type_index_src: va,
	});
	b.push(I::I32Const(types::TAG_DICT));
	b.push(I::LocalGet(NEW));
	b.push(I::StructNew(types::T_DICT));
	let locals = vec![
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
		ValType::I32,
		types::valarray_ref(),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__dict_map(dict, f) -> dict`: `f` over each value, keys preserved.
pub(crate) fn build_dict_map_fn(arity1: u32) -> Function {
	use Instruction as I;
	const D: u32 = 0;
	const F: u32 = 1;
	const ENTRIES: u32 = 2;
	const N: u32 = 3;
	const I_: u32 = 4;
	const NEW: u32 = 5;
	const K: u32 = 6;
	const NV: u32 = 7;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	let mut b: Vec<Instruction> = Vec::new();
	dict_entries_of(&mut b, D);
	b.push(I::LocalSet(ENTRIES));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::LocalGet(N));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(NEW));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	dict_entry_field(&mut b, ENTRIES, I_, 0);
	b.push(I::LocalSet(K));
	// NV = f(value): env = f, arg = value, call_indirect.
	b.push(I::LocalGet(F));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_CLOSURE)));
	dict_entry_field(&mut b, ENTRIES, I_, 1);
	b.push(I::LocalGet(F));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_CLOSURE)));
	b.push(I::StructGet {
		struct_type_index: types::T_CLOSURE,
		field_index: 1,
	});
	b.push(I::CallIndirect {
		type_index: arity1,
		table_index: 0,
	});
	b.push(I::LocalSet(NV));
	// NEW[i] = (K, NV).
	b.push(I::LocalGet(NEW));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(types::TAG_TUPLE));
	b.push(I::LocalGet(K));
	b.push(I::LocalGet(NV));
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
	b.push(I::LocalGet(NEW));
	b.push(I::StructNew(types::T_DICT));
	let locals = vec![
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
		types::valarray_ref(),
		types::value_ref(),
		types::value_ref(),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}

/// Build `__dict_filter(dict, f) -> dict`: keep entries where `f key value` is
/// true (the entry tuple is reused verbatim).
pub(crate) fn build_dict_filter_fn(arity2: u32) -> Function {
	use Instruction as I;
	const D: u32 = 0;
	const F: u32 = 1;
	const ENTRIES: u32 = 2;
	const N: u32 = 3;
	const I_: u32 = 4;
	const TMP: u32 = 5;
	const W: u32 = 6;
	const K: u32 = 7;
	const V: u32 = 8;
	const OUT: u32 = 9;
	let empty = wasm_encoder::BlockType::Empty;
	let va = types::T_VALARRAY;
	let mut b: Vec<Instruction> = Vec::new();
	dict_entries_of(&mut b, D);
	b.push(I::LocalSet(ENTRIES));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::LocalGet(N));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(TMP));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(W));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	dict_entry_field(&mut b, ENTRIES, I_, 0);
	b.push(I::LocalSet(K));
	dict_entry_field(&mut b, ENTRIES, I_, 1);
	b.push(I::LocalSet(V));
	// keep = f(k, v): env = f, args k v, call_indirect; unbox the $bool.
	b.push(I::LocalGet(F));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_CLOSURE)));
	b.push(I::LocalGet(K));
	b.push(I::LocalGet(V));
	b.push(I::LocalGet(F));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_CLOSURE)));
	b.push(I::StructGet {
		struct_type_index: types::T_CLOSURE,
		field_index: 1,
	});
	b.push(I::CallIndirect {
		type_index: arity2,
		table_index: 0,
	});
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_BOOL)));
	b.push(I::StructGet {
		struct_type_index: types::T_BOOL,
		field_index: 1,
	});
	b.push(I::If(empty));
	// TMP[W] = entry; W += 1.
	b.push(I::LocalGet(TMP));
	b.push(I::LocalGet(W));
	b.push(I::LocalGet(ENTRIES));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(va));
	b.push(I::ArraySet(va));
	b.push(I::LocalGet(W));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(W));
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
								 // OUT = array(W); copy TMP[0..W].
	b.push(I::LocalGet(W));
	b.push(I::ArrayNewDefault(va));
	b.push(I::LocalSet(OUT));
	b.push(I::LocalGet(OUT));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(TMP));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(W));
	b.push(I::ArrayCopy {
		array_type_index_dst: va,
		array_type_index_src: va,
	});
	b.push(I::I32Const(types::TAG_DICT));
	b.push(I::LocalGet(OUT));
	b.push(I::StructNew(types::T_DICT));
	let locals = vec![
		types::valarray_ref(),
		ValType::I32,
		ValType::I32,
		types::valarray_ref(),
		ValType::I32,
		types::value_ref(),
		types::value_ref(),
		types::valarray_ref(),
	];
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}
