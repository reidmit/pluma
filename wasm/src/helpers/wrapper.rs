// First-class builtin wrappers: the `(env, args…) -> value` closure bodies for a
// pure-compute builtin used as a method-dict method (`builtin_arity`,
// `build_builtin_wrapper`).

use wasm_encoder::*;

use crate::runtime::OrderingLits;
use crate::types;

/// The arity of a pure-compute builtin we can emit a wasm wrapper for, or `None`
/// if unsupported (string/bytes compare, hashes, … — later milestones).
pub(crate) fn builtin_arity(tag: &str) -> Option<usize> {
	Some(match tag {
		"int-add" | "int-sub" | "int-mul" | "int-div" | "float-add" | "float-sub" | "float-mul"
		| "float-div" | "int-compare" | "float-compare" | "string-compare" | "bytes-compare" => 2,
		"int-negate" | "float-negate" => 1,
		// `hash` instances: wrappable so a primitive `hash` method-dict can be
		// built, but the wasm `dict` scans with `__eq` and never calls hash, so the
		// wrapper body is unreachable (see `build_builtin_wrapper`).
		"int-hash" | "float-hash" | "string-hash" | "bool-hash" | "bytes-hash" => 1,
		_ => return None,
	})
}

/// Build the wasm wrapper for a pure-compute builtin used as a first-class value
/// (e.g. a `numeric`/`ord` dict method). Env-first closure convention: `(env,
/// args…) -> value`. Unboxes args, computes, reboxes. Comparisons return an
/// `ordering` variant; `ord` carries those variants' tags + interned display
/// names (resolved in `Module::build` when a `*-compare` wrapper is reachable).
pub(crate) fn build_builtin_wrapper(tag: &str, ord: &OrderingLits) -> Option<Function> {
	use Instruction as I;
	let arity = builtin_arity(tag)?;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	let getf = |t, f| I::StructGet {
		struct_type_index: t,
		field_index: f,
	};
	// Unbox arg local `n` (1-based) of scalar struct `ty` (field 1).
	let unbox = |b: &mut Vec<Instruction>, n: u32, ty: u32| {
		b.push(I::LocalGet(n));
		b.push(cast(ty));
		b.push(getf(ty, 1));
	};
	// Emit `return <ordering variant>` for the given within-enum tag + display
	// name (a 4-field `$variant` with an empty payload).
	let mk_ord = |b: &mut Vec<Instruction>, vtag: u32, (off, len): (u32, u32)| {
		b.push(I::I32Const(types::TAG_VARIANT));
		b.push(I::I32Const(vtag as i32));
		b.push(I::I32Const(types::TAG_STR));
		b.push(I::I32Const(off as i32));
		b.push(I::I32Const(len as i32));
		b.push(I::ArrayNewData {
			array_type_index: types::T_BYTES,
			array_data_index: 0,
		});
		b.push(I::StructNew(types::T_STR));
		b.push(I::ArrayNewFixed {
			array_type_index: types::T_VALARRAY,
			array_size: 0,
		});
		b.push(I::StructNew(types::T_VARIANT));
		b.push(I::Return);
	};
	let mut b: Vec<Instruction> = Vec::new();
	let mut extra_locals: Vec<ValType> = Vec::new();

	// Arithmetic: unbox both (or one), apply op, rebox. Result staged in a temp so
	// the box tag sits below it.
	let arith = |b: &mut Vec<Instruction>,
	             extra: &mut Vec<ValType>,
	             ty: u32,
	             tag_const: i32,
	             scalar: ValType,
	             op: Instruction<'static>,
	             unary: bool| {
		let tmp = (arity + 1) as u32; // first local past env+params
		extra.push(scalar);
		if unary {
			// negate: 0 - x  (int) / f64.neg (float)
			if scalar == ValType::I64 {
				b.push(I::I64Const(0));
				b.push(I::LocalGet(1));
				b.push(cast(ty));
				b.push(getf(ty, 1));
				b.push(I::I64Sub);
			} else {
				b.push(I::LocalGet(1));
				b.push(cast(ty));
				b.push(getf(ty, 1));
				b.push(I::F64Neg);
			}
		} else {
			b.push(I::LocalGet(1));
			b.push(cast(ty));
			b.push(getf(ty, 1));
			b.push(I::LocalGet(2));
			b.push(cast(ty));
			b.push(getf(ty, 1));
			b.push(op);
		}
		b.push(I::LocalSet(tmp));
		b.push(I::I32Const(tag_const));
		b.push(I::LocalGet(tmp));
		b.push(I::StructNew(ty));
	};

	match tag {
		"int-add" => arith(
			&mut b,
			&mut extra_locals,
			types::T_INT,
			types::TAG_INT,
			ValType::I64,
			I::I64Add,
			false,
		),
		"int-sub" => arith(
			&mut b,
			&mut extra_locals,
			types::T_INT,
			types::TAG_INT,
			ValType::I64,
			I::I64Sub,
			false,
		),
		"int-mul" => arith(
			&mut b,
			&mut extra_locals,
			types::T_INT,
			types::TAG_INT,
			ValType::I64,
			I::I64Mul,
			false,
		),
		"int-div" => arith(
			&mut b,
			&mut extra_locals,
			types::T_INT,
			types::TAG_INT,
			ValType::I64,
			I::I64DivS,
			false,
		),
		"int-negate" => arith(
			&mut b,
			&mut extra_locals,
			types::T_INT,
			types::TAG_INT,
			ValType::I64,
			I::Nop,
			true,
		),
		"float-add" => arith(
			&mut b,
			&mut extra_locals,
			types::T_FLOAT,
			types::TAG_FLOAT,
			ValType::F64,
			I::F64Add,
			false,
		),
		"float-sub" => arith(
			&mut b,
			&mut extra_locals,
			types::T_FLOAT,
			types::TAG_FLOAT,
			ValType::F64,
			I::F64Sub,
			false,
		),
		"float-mul" => arith(
			&mut b,
			&mut extra_locals,
			types::T_FLOAT,
			types::TAG_FLOAT,
			ValType::F64,
			I::F64Mul,
			false,
		),
		"float-div" => arith(
			&mut b,
			&mut extra_locals,
			types::T_FLOAT,
			types::TAG_FLOAT,
			ValType::F64,
			I::F64Div,
			false,
		),
		"float-negate" => arith(
			&mut b,
			&mut extra_locals,
			types::T_FLOAT,
			types::TAG_FLOAT,
			ValType::F64,
			I::Nop,
			true,
		),
		"int-compare" | "float-compare" => {
			let (ty, lt, eq) = if tag == "int-compare" {
				(types::T_INT, I::I64LtS, I::I64Eq)
			} else {
				(types::T_FLOAT, I::F64Lt, I::F64Eq)
			};
			// a < b -> lt
			unbox(&mut b, 1, ty);
			unbox(&mut b, 2, ty);
			b.push(lt);
			b.push(I::If(wasm_encoder::BlockType::Empty));
			mk_ord(&mut b, ord.lt_tag, ord.lt_name);
			b.push(I::End);
			// a == b -> eq
			unbox(&mut b, 1, ty);
			unbox(&mut b, 2, ty);
			b.push(eq);
			b.push(I::If(wasm_encoder::BlockType::Empty));
			mk_ord(&mut b, ord.eq_tag, ord.eq_name);
			b.push(I::End);
			// else gt
			mk_ord(&mut b, ord.gt_tag, ord.gt_name);
		}
		// String / bytes ordering: lexicographic byte compare (Rust `str`/`[u8]`
		// `Ord` is byte-lexicographic). Both reuse the `$str` `{tag, $bytes}` shape,
		// so the same loop serves either. Locals past env+2 args: abytes, bbytes
		// ($bytes), alen, blen, minlen, i, av, bv (i32).
		"string-compare" | "bytes-compare" => {
			extra_locals.push(types::bytes_ref()); // 3 abytes
			extra_locals.push(types::bytes_ref()); // 4 bbytes
			extra_locals.push(ValType::I32); // 5 alen
			extra_locals.push(ValType::I32); // 6 blen
			extra_locals.push(ValType::I32); // 7 minlen
			extra_locals.push(ValType::I32); // 8 i
			const ABYTES: u32 = 3;
			const BBYTES: u32 = 4;
			const ALEN: u32 = 5;
			const BLEN: u32 = 6;
			const MINLEN: u32 = 7;
			const I_: u32 = 8;
			let empty = wasm_encoder::BlockType::Empty;
			// abytes = (cast $str a).field1; bbytes likewise.
			b.push(I::LocalGet(1));
			b.push(cast(types::T_STR));
			b.push(getf(types::T_STR, 1));
			b.push(I::LocalSet(ABYTES));
			b.push(I::LocalGet(2));
			b.push(cast(types::T_STR));
			b.push(getf(types::T_STR, 1));
			b.push(I::LocalSet(BBYTES));
			// alen / blen / minlen.
			b.push(I::LocalGet(ABYTES));
			b.push(I::ArrayLen);
			b.push(I::LocalSet(ALEN));
			b.push(I::LocalGet(BBYTES));
			b.push(I::ArrayLen);
			b.push(I::LocalSet(BLEN));
			b.push(I::LocalGet(ALEN));
			b.push(I::LocalGet(BLEN));
			b.push(I::I32LtU);
			b.push(I::If(wasm_encoder::BlockType::Result(ValType::I32)));
			b.push(I::LocalGet(ALEN));
			b.push(I::Else);
			b.push(I::LocalGet(BLEN));
			b.push(I::End);
			b.push(I::LocalSet(MINLEN));
			// i = 0; scan the shared prefix.
			b.push(I::I32Const(0));
			b.push(I::LocalSet(I_));
			b.push(I::Block(empty)); // $done
			b.push(I::Loop(empty)); // $cmp
			b.push(I::LocalGet(I_));
			b.push(I::LocalGet(MINLEN));
			b.push(I::I32GeU);
			b.push(I::BrIf(1)); // -> $done
											 // av < bv -> less ; av > bv -> greater (unsigned byte compare).
			b.push(I::LocalGet(ABYTES));
			b.push(I::LocalGet(I_));
			b.push(I::ArrayGetU(types::T_BYTES));
			b.push(I::LocalGet(BBYTES));
			b.push(I::LocalGet(I_));
			b.push(I::ArrayGetU(types::T_BYTES));
			b.push(I::I32LtU);
			b.push(I::If(empty));
			mk_ord(&mut b, ord.lt_tag, ord.lt_name);
			b.push(I::End);
			b.push(I::LocalGet(ABYTES));
			b.push(I::LocalGet(I_));
			b.push(I::ArrayGetU(types::T_BYTES));
			b.push(I::LocalGet(BBYTES));
			b.push(I::LocalGet(I_));
			b.push(I::ArrayGetU(types::T_BYTES));
			b.push(I::I32GtU);
			b.push(I::If(empty));
			mk_ord(&mut b, ord.gt_tag, ord.gt_name);
			b.push(I::End);
			b.push(I::LocalGet(I_));
			b.push(I::I32Const(1));
			b.push(I::I32Add);
			b.push(I::LocalSet(I_));
			b.push(I::Br(0)); // -> $cmp
			b.push(I::End); // loop
			b.push(I::End); // block $done
									 // Prefix equal: shorter sorts first.
			b.push(I::LocalGet(ALEN));
			b.push(I::LocalGet(BLEN));
			b.push(I::I32LtU);
			b.push(I::If(empty));
			mk_ord(&mut b, ord.lt_tag, ord.lt_name);
			b.push(I::End);
			b.push(I::LocalGet(ALEN));
			b.push(I::LocalGet(BLEN));
			b.push(I::I32GtU);
			b.push(I::If(empty));
			mk_ord(&mut b, ord.gt_tag, ord.gt_name);
			b.push(I::End);
			mk_ord(&mut b, ord.eq_tag, ord.eq_name);
		}
		// `hash` wrappers exist only so a primitive `hash` method-dict can be
		// materialized; the wasm `dict` never calls them (it scans keys with
		// `__eq`). A trap keeps it honest if some future caller does reach one.
		"int-hash" | "float-hash" | "string-hash" | "bool-hash" | "bytes-hash" => {
			b.push(I::Unreachable);
		}
		_ => return None,
	}

	// `extra_locals` are the locals past the env+arg params (which come from the
	// function's declared type, not declared here).
	let mut f = Function::new_with_locals_types(extra_locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	Some(f)
}
