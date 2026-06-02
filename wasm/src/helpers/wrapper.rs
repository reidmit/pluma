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

	// Unbox arg param `n` (1-based) of scalar struct `ty` (field 1). An `int` may be
	// an `i31ref` immediate, so it routes through `unbox_int`; float/bool/str are
	// always heap structs.
	let unbox = |w: &mut Wat, n: u32, ty: u32| {
		let arg = w.param(n);
		if ty == types::T_INT {
			w.local_get(arg).unbox_int();
		} else {
			w.local_get(arg).ref_cast(ty).struct_get(ty, 1);
		}
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
			// An `int` operand/result rides as an `i31ref` (small) or heap `$int`, so
			// unbox/box through `unbox_int`/`box_int`; `float` is always a heap `$float`.
			let is_int = scalar == ValType::I64;
			let tmp = w.local(scalar); // first local past env+params
			let unbox_arg = |w: &mut Wat, a| {
				if is_int {
					w.local_get(a).unbox_int();
				} else {
					w.local_get(a).ref_cast(ty).struct_get(ty, 1);
				}
			};
			if unary {
				// negate: 0 - x (int) / f64.neg (float).
				let a1 = w.param(1);
				if is_int {
					w.i64(0);
					unbox_arg(w, a1);
					w.i64_sub();
				} else {
					unbox_arg(w, a1);
					w.f64_neg();
				}
			} else {
				let (a1, a2) = (w.param(1), w.param(2));
				unbox_arg(w, a1);
				unbox_arg(w, a2);
				op(w);
			}
			w.local_set(tmp);
			if is_int {
				w.local_get(tmp).box_int();
			} else {
				w.i32(tag_const).local_get(tmp).struct_new(ty);
			}
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
		// `hash` instances. The wasm `dict` scans keys with `__eq` and never calls
		// these, but a program can call `hash.hash x` directly (and parametric
		// instances recurse into the primitive ones), so they compute the real value
		// — matching `vm::value::primitive_hash` EXACTLY. All box their result `$int`.
		//
		// int: the value itself. float: the f64 bit pattern as i64. bool: 0/1.
		"int-hash" => {
			// hash(int) == the int; the boxed `$int` arg is already that.
			let p1 = w.param(1);
			w.local_get(p1);
		}
		"float-hash" => {
			w.i32(types::TAG_INT);
			unbox(&mut w, 1, types::T_FLOAT);
			w.i64_reinterpret_f64();
			w.struct_new(types::T_INT);
		}
		"bool-hash" => {
			w.i32(types::TAG_INT);
			unbox(&mut w, 1, types::T_BOOL);
			w.i64_extend_i32_u();
			w.struct_new(types::T_INT);
		}
		// string / bytes: FNV-1a (64-bit) over the `$bytes` backing — a defined,
		// portable hash (the VM uses the same two constants in `value::fnv1a`).
		// Both share the `$str` `{tag, $bytes}` shape, so one loop serves either.
		"string-hash" | "bytes-hash" => {
			const FNV_OFFSET: i64 = 0xcbf2_9ce4_8422_2325u64 as i64;
			const FNV_PRIME: i64 = 0x0000_0100_0000_01b3;
			let bytes = w.local(types::bytes_ref());
			let len = w.local(ValType::I32);
			let i = w.local(ValType::I32);
			let h = w.local(ValType::I64);
			let p1 = w.param(1);
			w.local_get(p1)
				.ref_cast(types::T_STR)
				.struct_get(types::T_STR, 1)
				.local_set(bytes);
			w.local_get(bytes).array_len().local_set(len);
			w.i32(0).local_set(i);
			w.i64(FNV_OFFSET).local_set(h);
			w.block("done", |w| {
				w.loop_("loop", |w| {
					w.local_get(i).local_get(len).i32_ge_u().br_if("done");
					// h = (h ^ byte) * PRIME.
					w.local_get(h);
					w.local_get(bytes).local_get(i).array_get_u(types::T_BYTES);
					w.i64_extend_i32_u();
					w.i64_xor();
					w.i64(FNV_PRIME);
					w.i64_mul();
					w.local_set(h);
					w.local_get(i).i32(1).i32_add().local_set(i);
					w.br("loop");
				});
			});
			w.i32(types::TAG_INT).local_get(h).struct_new(types::T_INT);
		}
		_ => return None,
	}

	Some(w.finish())
}

/// Build the wasm wrapper for a single-arg host-import builtin used as a
/// first-class value — e.g. `print` passed to `list.each xs print`. Env-first
/// closure convention `(env, arg) -> value`: forward the boxed arg to the host
/// import (`host_idx`), then return `nothing` (these imports — print/io writers
/// /io.fail — produce no value). A bare builtin has no runtime `$value`, so this
/// is what a `MakeClosure` over the builtin lowers to.
pub(crate) fn build_host_value_wrapper(host_idx: u32) -> Function {
	let mut w = Wat::new(2);
	let arg = w.param(1); // param 0 is the (ignored) env.
	w.local_get(arg);
	w.call(host_idx);
	w.i32(types::TAG_NOTHING).struct_new(types::T_VALUE);
	w.finish()
}
