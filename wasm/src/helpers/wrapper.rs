// First-class builtin wrappers: the `(env, args…) -> value` closure bodies for a
// pure-compute builtin used as a method-dict method (`builtin_arity`,
// `build_builtin_wrapper`).

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::Wat;
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
	let arity = builtin_arity(tag)?;
	// Params: env (slot 0) then `arity` boxed args (slots 1..=arity).
	let mut w = Wat::new(arity as u32 + 1);

	// Unbox arg param `n` (1-based) of scalar struct `ty` (field 1).
	let unbox = |w: &mut Wat, n: u32, ty: u32| {
		let arg = w.param(n);
		w.local_get(arg).ref_cast(ty).struct_get(ty, 1);
	};
	// Emit `return <ordering variant>` for the given within-enum tag + display name
	// (a 4-field `$variant` with an empty payload).
	let mk_ord = |w: &mut Wat, vtag: u32, (off, len): (u32, u32)| {
		w.i32(types::TAG_VARIANT);
		w.i32(vtag as i32);
		w.i32(types::TAG_STR);
		w.i32(off as i32);
		w.i32(len as i32);
		w.array_new_data(types::T_BYTES, 0);
		w.struct_new(types::T_STR);
		w.array_new_fixed(types::T_VALARRAY, 0);
		w.struct_new(types::T_VARIANT);
		w.ret();
	};

	// Arithmetic: unbox both (or one), apply `op`, rebox. Result staged in a temp so
	// the box tag sits below it. `op` runs with both unboxed scalars on the stack.
	let arith =
		|w: &mut Wat, scalar: ValType, ty: u32, tag_const: i32, op: fn(&mut Wat), unary: bool| {
			let tmp = w.local(scalar); // first local past env+params
			if unary {
				// negate: 0 - x (int) / f64.neg (float).
				let a1 = w.param(1);
				if scalar == ValType::I64 {
					w.i64(0);
					w.local_get(a1).ref_cast(ty).struct_get(ty, 1);
					w.i64_sub();
				} else {
					w.local_get(a1).ref_cast(ty).struct_get(ty, 1);
					w.f64_neg();
				}
			} else {
				let (a1, a2) = (w.param(1), w.param(2));
				w.local_get(a1).ref_cast(ty).struct_get(ty, 1);
				w.local_get(a2).ref_cast(ty).struct_get(ty, 1);
				op(w);
			}
			w.local_set(tmp);
			w.i32(tag_const).local_get(tmp).struct_new(ty);
		};

	match tag {
		"int-add" => arith(
			&mut w,
			ValType::I64,
			types::T_INT,
			types::TAG_INT,
			|w| {
				w.i64_add();
			},
			false,
		),
		"int-sub" => arith(
			&mut w,
			ValType::I64,
			types::T_INT,
			types::TAG_INT,
			|w| {
				w.i64_sub();
			},
			false,
		),
		"int-mul" => arith(
			&mut w,
			ValType::I64,
			types::T_INT,
			types::TAG_INT,
			|w| {
				w.i64_mul();
			},
			false,
		),
		"int-div" => arith(
			&mut w,
			ValType::I64,
			types::T_INT,
			types::TAG_INT,
			|w| {
				w.i64_div_s();
			},
			false,
		),
		"int-negate" => arith(
			&mut w,
			ValType::I64,
			types::T_INT,
			types::TAG_INT,
			|_| {},
			true,
		),
		"float-add" => arith(
			&mut w,
			ValType::F64,
			types::T_FLOAT,
			types::TAG_FLOAT,
			|w| {
				w.f64_add();
			},
			false,
		),
		"float-sub" => arith(
			&mut w,
			ValType::F64,
			types::T_FLOAT,
			types::TAG_FLOAT,
			|w| {
				w.f64_sub();
			},
			false,
		),
		"float-mul" => arith(
			&mut w,
			ValType::F64,
			types::T_FLOAT,
			types::TAG_FLOAT,
			|w| {
				w.f64_mul();
			},
			false,
		),
		"float-div" => arith(
			&mut w,
			ValType::F64,
			types::T_FLOAT,
			types::TAG_FLOAT,
			|w| {
				w.f64_div();
			},
			false,
		),
		"float-negate" => arith(
			&mut w,
			ValType::F64,
			types::T_FLOAT,
			types::TAG_FLOAT,
			|_| {},
			true,
		),
		"int-compare" | "float-compare" => {
			let (ty, lt, eq): (u32, fn(&mut Wat), fn(&mut Wat)) = if tag == "int-compare" {
				(
					types::T_INT,
					|w| {
						w.i64_lt_s();
					},
					|w| {
						w.i64_eq();
					},
				)
			} else {
				(
					types::T_FLOAT,
					|w| {
						w.f64_lt();
					},
					|w| {
						w.f64_eq();
					},
				)
			};
			// a < b -> lt.
			unbox(&mut w, 1, ty);
			unbox(&mut w, 2, ty);
			lt(&mut w);
			w.if_(|w| mk_ord(w, ord.lt_tag, ord.lt_name));
			// a == b -> eq.
			unbox(&mut w, 1, ty);
			unbox(&mut w, 2, ty);
			eq(&mut w);
			w.if_(|w| mk_ord(w, ord.eq_tag, ord.eq_name));
			// else gt.
			mk_ord(&mut w, ord.gt_tag, ord.gt_name);
		}
		// String / bytes ordering: lexicographic byte compare (Rust `str`/`[u8]`
		// `Ord` is byte-lexicographic). Both reuse the `$str` `{tag, $bytes}` shape,
		// so the same loop serves either.
		"string-compare" | "bytes-compare" => {
			let abytes = w.local(types::bytes_ref());
			let bbytes = w.local(types::bytes_ref());
			let alen = w.local(ValType::I32);
			let blen = w.local(ValType::I32);
			let minlen = w.local(ValType::I32);
			let i = w.local(ValType::I32);
			let (a1, a2) = (w.param(1), w.param(2));
			// abytes / bbytes = the `$bytes` backing of each operand.
			w.local_get(a1)
				.ref_cast(types::T_STR)
				.struct_get(types::T_STR, 1)
				.local_set(abytes);
			w.local_get(a2)
				.ref_cast(types::T_STR)
				.struct_get(types::T_STR, 1)
				.local_set(bbytes);
			// alen / blen / minlen.
			w.local_get(abytes).array_len().local_set(alen);
			w.local_get(bbytes).array_len().local_set(blen);
			w.local_get(alen).local_get(blen).i32_lt_u();
			w.if_result(
				ValType::I32,
				|w| {
					w.local_get(alen);
				},
				|w| {
					w.local_get(blen);
				},
			);
			w.local_set(minlen);
			// i = 0; scan the shared prefix.
			w.i32(0).local_set(i);
			w.block("done", |w| {
				w.loop_("cmp", |w| {
					w.local_get(i).local_get(minlen).i32_ge_u().br_if("done");
					// av < bv -> less ; av > bv -> greater (unsigned byte compare).
					w.local_get(abytes).local_get(i).array_get_u(types::T_BYTES);
					w.local_get(bbytes).local_get(i).array_get_u(types::T_BYTES);
					w.i32_lt_u();
					w.if_(|w| mk_ord(w, ord.lt_tag, ord.lt_name));
					w.local_get(abytes).local_get(i).array_get_u(types::T_BYTES);
					w.local_get(bbytes).local_get(i).array_get_u(types::T_BYTES);
					w.i32_gt_u();
					w.if_(|w| mk_ord(w, ord.gt_tag, ord.gt_name));
					w.local_get(i).i32(1).i32_add().local_set(i);
					w.br("cmp");
				});
			});
			// Prefix equal: shorter sorts first.
			w.local_get(alen).local_get(blen).i32_lt_u();
			w.if_(|w| mk_ord(w, ord.lt_tag, ord.lt_name));
			w.local_get(alen).local_get(blen).i32_gt_u();
			w.if_(|w| mk_ord(w, ord.gt_tag, ord.gt_name));
			mk_ord(&mut w, ord.eq_tag, ord.eq_name);
		}
		// `hash` wrappers exist only so a primitive `hash` method-dict can be
		// materialized; the wasm `dict` never calls them (it scans keys with `__eq`).
		// A trap keeps it honest if some future caller does reach one.
		"int-hash" | "float-hash" | "string-hash" | "bool-hash" | "bytes-hash" => {
			w.unreachable();
		}
		_ => return None,
	}

	Some(w.finish())
}
