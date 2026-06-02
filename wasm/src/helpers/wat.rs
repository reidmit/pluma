// A small WAT-flavored assembler for the synthetic runtime helpers.
//
// The helpers used to be authored as a flat `Vec<Instruction>` with `const FOO:
// u32` local indices and bare `Br(2)` branch depths — correct, but hard to read.
// `Wat` emits the exact same `wasm_encoder::Instruction`s while letting the source
// read like the WebAssembly text format:
//
//   * locals are named handles (`let n = w.local(I32)`) instead of `const N: u32`;
//   * control flow nests with closures (`w.block("brk", |w| { … })`), so the shape
//     is visible and the matching `End`s are emitted automatically;
//   * branches name their target label (`w.br_if("brk")`) and the relative depth is
//     computed from the stack of open labels — no more magic `Br(2)` with a comment.
//
// Every dynamic index a helper needs — dependency function indices, interned
// `call_indirect` type indices, wire scratch globals, data-segment offsets — is
// still a plain Rust value threaded straight through to the matching instruction,
// so `Wat` adds no indirection or table over the old hand-built functions: it's the
// same bytes, authored legibly.

use wasm_encoder::{BlockType, Function, HeapType, Instruction, ValType};

use Instruction as I;

use crate::types;

/// A local (or param) slot, returned by [`Wat::param`]/[`Wat::local`] so callers
/// reference locals by name rather than a hand-tracked `u32`.
#[derive(Clone, Copy)]
pub(crate) struct Local(u32);

/// A function body under construction. Params occupy indices `0..n_params`; each
/// [`local`](Wat::local) appends after them. Instructions accumulate in order; the
/// open-label stack drives branch-depth resolution.
pub(crate) struct Wat {
	n_params: u32,
	locals: Vec<ValType>,
	instrs: Vec<Instruction<'static>>,
	/// One entry per open `block`/`loop`/`if` (`None` = anonymous `if`/`if_else`),
	/// innermost last. `br`/`br_if` scan it from the top to turn a label into the
	/// relative depth wasm wants.
	labels: Vec<Option<&'static str>>,
}

impl Wat {
	/// A new body with `n_params` boxed params (their types come from the helper's
	/// declared function type, so they aren't re-listed here).
	pub(crate) fn new(n_params: u32) -> Self {
		Wat {
			n_params,
			locals: Vec::new(),
			instrs: Vec::new(),
			labels: Vec::new(),
		}
	}

	/// The `i`-th param slot.
	pub(crate) fn param(&self, i: u32) -> Local {
		debug_assert!(i < self.n_params, "param {i} out of range");
		Local(i)
	}

	/// Declare a fresh local of `ty`, returning its slot.
	pub(crate) fn local(&mut self, ty: ValType) -> Local {
		let idx = self.n_params + self.locals.len() as u32;
		self.locals.push(ty);
		Local(idx)
	}

	/// Finish the body: realize the locals, replay the instructions, and close the
	/// implicit function block with the trailing `End`.
	pub(crate) fn finish(self) -> Function {
		debug_assert!(self.labels.is_empty(), "unbalanced control flow");
		let mut f = Function::new_with_locals_types(self.locals);
		for ins in &self.instrs {
			f.instruction(ins);
		}
		f.instruction(&I::End);
		f
	}

	// ---- internals -------------------------------------------------------------

	fn push(&mut self, ins: Instruction<'static>) -> &mut Self {
		self.instrs.push(ins);
		self
	}

	/// Relative depth of the open label nearest the top of the stack.
	fn depth_of(&self, label: &str) -> u32 {
		self
			.labels
			.iter()
			.rev()
			.position(|l| *l == Some(label))
			.unwrap_or_else(|| panic!("branch to unopened label `{label}`")) as u32
	}

	// ---- control flow ----------------------------------------------------------

	/// `(block … end)` labelled `label`; a `br`/`br_if` to it exits the block.
	pub(crate) fn block(&mut self, label: &'static str, body: impl FnOnce(&mut Self)) -> &mut Self {
		self.push(I::Block(BlockType::Empty));
		self.labels.push(Some(label));
		body(&mut *self);
		self.labels.pop();
		self.push(I::End)
	}

	/// `(loop … end)` labelled `label`; a `br`/`br_if` to it jumps back to the top.
	pub(crate) fn loop_(&mut self, label: &'static str, body: impl FnOnce(&mut Self)) -> &mut Self {
		self.push(I::Loop(BlockType::Empty));
		self.labels.push(Some(label));
		body(&mut *self);
		self.labels.pop();
		self.push(I::End)
	}

	/// `(if … end)` with no else — runs `body` when the popped i32 is non-zero.
	pub(crate) fn if_(&mut self, body: impl FnOnce(&mut Self)) -> &mut Self {
		self.push(I::If(BlockType::Empty));
		self.labels.push(None);
		body(&mut *self);
		self.labels.pop();
		self.push(I::End)
	}

	/// `(if … else … end)`, both arms valueless.
	pub(crate) fn if_else(
		&mut self,
		then: impl FnOnce(&mut Self),
		els: impl FnOnce(&mut Self),
	) -> &mut Self {
		self.push(I::If(BlockType::Empty));
		self.labels.push(None);
		then(&mut *self);
		self.push(I::Else);
		els(&mut *self);
		self.labels.pop();
		self.push(I::End)
	}

	/// `(if (result ty) … else … end)` — each arm leaves one `ty` value on the stack.
	pub(crate) fn if_result(
		&mut self,
		ty: ValType,
		then: impl FnOnce(&mut Self),
		els: impl FnOnce(&mut Self),
	) -> &mut Self {
		self.push(I::If(BlockType::Result(ty)));
		self.labels.push(None);
		then(&mut *self);
		self.push(I::Else);
		els(&mut *self);
		self.labels.pop();
		self.push(I::End)
	}

	/// `br` to `label` (unconditional).
	pub(crate) fn br(&mut self, label: &str) -> &mut Self {
		let d = self.depth_of(label);
		self.push(I::Br(d))
	}

	/// `br_if` to `label` (branch when the popped i32 is non-zero).
	pub(crate) fn br_if(&mut self, label: &str) -> &mut Self {
		let d = self.depth_of(label);
		self.push(I::BrIf(d))
	}

	// ---- locals / globals / consts ---------------------------------------------

	pub(crate) fn local_get(&mut self, l: Local) -> &mut Self {
		self.push(I::LocalGet(l.0))
	}
	pub(crate) fn local_set(&mut self, l: Local) -> &mut Self {
		self.push(I::LocalSet(l.0))
	}
	/// `local.tee` — set the local and leave the value on the stack.
	pub(crate) fn local_tee(&mut self, l: Local) -> &mut Self {
		self.push(I::LocalTee(l.0))
	}
	pub(crate) fn global_get(&mut self, g: u32) -> &mut Self {
		self.push(I::GlobalGet(g))
	}
	pub(crate) fn global_set(&mut self, g: u32) -> &mut Self {
		self.push(I::GlobalSet(g))
	}
	pub(crate) fn i32(&mut self, v: i32) -> &mut Self {
		self.push(I::I32Const(v))
	}
	pub(crate) fn i64(&mut self, v: i64) -> &mut Self {
		self.push(I::I64Const(v))
	}

	// ---- calls -----------------------------------------------------------------

	pub(crate) fn call(&mut self, func: u32) -> &mut Self {
		self.push(I::Call(func))
	}
	/// `call_indirect` through table 0 with the given function type.
	pub(crate) fn call_indirect(&mut self, type_index: u32) -> &mut Self {
		self.push(I::CallIndirect {
			type_index,
			table_index: 0,
		})
	}

	// ---- GC structs ------------------------------------------------------------

	pub(crate) fn struct_get(&mut self, ty: u32, field: u32) -> &mut Self {
		self.push(I::StructGet {
			struct_type_index: ty,
			field_index: field,
		})
	}
	pub(crate) fn struct_set(&mut self, ty: u32, field: u32) -> &mut Self {
		self.push(I::StructSet {
			struct_type_index: ty,
			field_index: field,
		})
	}
	pub(crate) fn struct_new(&mut self, ty: u32) -> &mut Self {
		self.push(I::StructNew(ty))
	}

	// ---- GC arrays -------------------------------------------------------------

	pub(crate) fn array_get(&mut self, ty: u32) -> &mut Self {
		self.push(I::ArrayGet(ty))
	}
	/// Unsigned element read — for the packed `i8` `$bytes` array.
	pub(crate) fn array_get_u(&mut self, ty: u32) -> &mut Self {
		self.push(I::ArrayGetU(ty))
	}
	pub(crate) fn array_set(&mut self, ty: u32) -> &mut Self {
		self.push(I::ArraySet(ty))
	}
	pub(crate) fn array_new_default(&mut self, ty: u32) -> &mut Self {
		self.push(I::ArrayNewDefault(ty))
	}
	pub(crate) fn array_new_fixed(&mut self, ty: u32, size: u32) -> &mut Self {
		self.push(I::ArrayNewFixed {
			array_type_index: ty,
			array_size: size,
		})
	}
	pub(crate) fn array_new_data(&mut self, ty: u32, data: u32) -> &mut Self {
		self.push(I::ArrayNewData {
			array_type_index: ty,
			array_data_index: data,
		})
	}
	pub(crate) fn array_copy(&mut self, dst: u32, src: u32) -> &mut Self {
		self.push(I::ArrayCopy {
			array_type_index_dst: dst,
			array_type_index_src: src,
		})
	}

	/// Manual element-copy loop: `dst[dst_off + k] = src[src_off + k]` for
	/// `k` in `0..len` (a `None` offset means 0). Use this instead of
	/// `array.copy`: wasmtime's `array.copy` libcall is a trap at every element
	/// type — ~90x slower than this loop on `$valarray` (GC-reference) arrays,
	/// and ~19x slower even on packed `$bytes` (so `__bytesconcat` open-codes the
	/// same loop rather than calling here, since it needs `array.get_u`).
	/// Allocates one scratch i32 local.
	pub(crate) fn copy_loop(
		&mut self,
		ty: u32,
		dst: Local,
		dst_off: Option<Local>,
		src: Local,
		src_off: Option<Local>,
		len: Local,
	) -> &mut Self {
		let k = self.local(ValType::I32);
		self.i32(0).local_set(k);
		self.block("cp_brk", |w| {
			w.loop_("cp_lp", |w| {
				w.local_get(k).local_get(len).i32_ge_s().br_if("cp_brk");
				// dst, then the destination index (dst_off + k, or k).
				w.local_get(dst);
				match dst_off {
					Some(o) => {
						w.local_get(o).local_get(k).i32_add();
					}
					None => {
						w.local_get(k);
					}
				}
				// src[src_off + k] — the value to store.
				w.local_get(src);
				match src_off {
					Some(o) => {
						w.local_get(o).local_get(k).i32_add();
					}
					None => {
						w.local_get(k);
					}
				}
				w.array_get(ty);
				w.array_set(ty);
				w.local_get(k).i32(1).i32_add().local_set(k);
				w.br("cp_lp");
			});
		});
		self
	}

	/// [`copy_loop`] for a packed `$bytes` array (`ty` is its type index). Packed
	/// fields require the unsigned `array.get_u` accessor — plain `array.get` is a
	/// validation error — so this can't share `copy_loop`'s body. The reason to
	/// loop rather than `array.copy` is the same and not specific to reference
	/// arrays: wasmtime's `array.copy` libcall is ~19x slower than this inline loop
	/// even on bytes. Allocates one scratch i32 local.
	pub(crate) fn copy_loop_bytes(
		&mut self,
		ty: u32,
		dst: Local,
		dst_off: Option<Local>,
		src: Local,
		src_off: Option<Local>,
		len: Local,
	) -> &mut Self {
		let k = self.local(ValType::I32);
		self.i32(0).local_set(k);
		self.block("cpb_brk", |w| {
			w.loop_("cpb_lp", |w| {
				w.local_get(k).local_get(len).i32_ge_s().br_if("cpb_brk");
				w.local_get(dst);
				match dst_off {
					Some(o) => {
						w.local_get(o).local_get(k).i32_add();
					}
					None => {
						w.local_get(k);
					}
				}
				w.local_get(src);
				match src_off {
					Some(o) => {
						w.local_get(o).local_get(k).i32_add();
					}
					None => {
						w.local_get(k);
					}
				}
				w.array_get_u(ty);
				w.array_set(ty);
				w.local_get(k).i32(1).i32_add().local_set(k);
				w.br("cpb_lp");
			});
		});
		self
	}

	// ---- references ------------------------------------------------------------

	/// `ref.cast (ref $ty)` — a non-null downcast to concrete type `ty`.
	pub(crate) fn ref_cast(&mut self, ty: u32) -> &mut Self {
		self.push(I::RefCastNonNull(HeapType::Concrete(ty)))
	}
	/// `ref.null $ty`.
	pub(crate) fn ref_null(&mut self, ty: u32) -> &mut Self {
		self.push(I::RefNull(HeapType::Concrete(ty)))
	}

	// ---- value discriminant / scalar boxing (i31-aware; see `notes/I31.md`) -----

	/// Read the runtime discriminant tag of a boxed value (`eqref` on the stack ->
	/// i32 tag). A small int is an `i31ref` immediate carrying no tag struct, so its
	/// kind is `TAG_INT`; any other value is a heap `$value` whose tag is field 0.
	pub(crate) fn value_tag(&mut self) -> &mut Self {
		let t = self.local(types::value_ref());
		self.local_set(t);
		// `nothing` is a null reference (no allocation); a small int is an `i31ref`
		// immediate; anything else is a heap `$value` whose tag is field 0.
		self.local_get(t).ref_is_null();
		self.if_result(
			ValType::I32,
			|w| {
				w.i32(types::TAG_NOTHING);
			},
			|w| {
				w.local_get(t).ref_test_i31();
				w.if_result(
					ValType::I32,
					|w| {
						w.i32(types::TAG_INT);
					},
					|w| {
						w.local_get(t)
							.ref_cast(types::T_VALUE)
							.struct_get(types::T_VALUE, 0);
					},
				);
			},
		)
	}

	/// Unbox a boxed int (`eqref` on the stack -> i64). A small int rides as an
	/// `i31ref` immediate (sign-extended); otherwise it's a heap `$int` (field 1).
	pub(crate) fn unbox_int(&mut self) -> &mut Self {
		let t = self.local(types::value_ref());
		self.local_set(t);
		self.local_get(t).ref_test_i31();
		self.if_result(
			ValType::I64,
			|w| {
				w.local_get(t).ref_cast_i31().i31_get_s().i64_extend_i32_s();
			},
			|w| {
				w.local_get(t)
					.ref_cast(types::T_INT)
					.struct_get(types::T_INT, 1);
			},
		)
	}

	/// Box an i64 (stack top) into an `eqref`: a value that fits in a signed 31-bit
	/// `i31ref` becomes that immediate (no allocation, not refcounted by DRC), else a
	/// heap `$int`. Allocates one scratch i64 local.
	pub(crate) fn box_int(&mut self) -> &mut Self {
		let t = self.local(ValType::I64);
		self.local_set(t);
		// fits a signed 31-bit i31? -2^30 <= v < 2^30
		self.local_get(t).i64(-(1 << 30)).i64_ge_s();
		self.local_get(t).i64(1 << 30).i64_lt_s();
		self.i32_and();
		self.if_result(
			types::value_ref(),
			|w| {
				w.local_get(t).i32_wrap_i64().ref_i31();
			},
			|w| {
				w.i32(types::TAG_INT).local_get(t).struct_new(types::T_INT);
			},
		)
	}

	/// `ref.test (ref i31)` — 1 if the value is a small-int immediate, else 0.
	pub(crate) fn ref_test_i31(&mut self) -> &mut Self {
		self.push(I::RefTestNonNull(HeapType::I31))
	}
	/// `ref.cast (ref i31)` — assert + narrow to the i31 immediate.
	pub(crate) fn ref_cast_i31(&mut self) -> &mut Self {
		self.push(I::RefCastNonNull(HeapType::I31))
	}
}

/// Generate the nullary opcode methods (no immediates) — each pushes one
/// instruction and returns `&mut Self` so they chain into a WAT-like line.
macro_rules! nullary {
	($($method:ident => $variant:ident,)*) => {
		impl Wat {
			$(
				#[inline]
				pub(crate) fn $method(&mut self) -> &mut Self {
					self.push(I::$variant)
				}
			)*
		}
	};
}

nullary! {
	// i32 arithmetic / bitwise
	i32_add => I32Add, i32_sub => I32Sub, i32_mul => I32Mul,
	i32_and => I32And, i32_or => I32Or, i32_shl => I32Shl, i32_shr_u => I32ShrU,
	// i32 comparisons
	i32_eq => I32Eq, i32_ne => I32Ne, i32_eqz => I32Eqz,
	i32_ge_s => I32GeS, i32_ge_u => I32GeU, i32_gt_s => I32GtS, i32_gt_u => I32GtU,
	i32_lt_s => I32LtS, i32_lt_u => I32LtU, i32_le_s => I32LeS,
	i32_wrap_i64 => I32WrapI64,
	// i64 arithmetic / bitwise
	i64_add => I64Add, i64_sub => I64Sub, i64_mul => I64Mul,
	i64_and => I64And, i64_or => I64Or, i64_xor => I64Xor,
	i64_shl => I64Shl, i64_shr_u => I64ShrU, i64_shr_s => I64ShrS,
	i64_div_s => I64DivS, i64_rem_s => I64RemS,
	// i64 comparisons / conversions
	i64_eq => I64Eq, i64_eqz => I64Eqz, i64_lt_s => I64LtS, i64_ge_s => I64GeS,
	i64_extend_i32_s => I64ExtendI32S, i64_extend_i32_u => I64ExtendI32U,
	i64_reinterpret_f64 => I64ReinterpretF64,
	// f64
	f64_add => F64Add, f64_sub => F64Sub, f64_mul => F64Mul, f64_div => F64Div,
	f64_neg => F64Neg, f64_lt => F64Lt, f64_eq => F64Eq,
	f64_reinterpret_i64 => F64ReinterpretI64,
	// arrays / references
	array_len => ArrayLen, ref_eq => RefEq, ref_is_null => RefIsNull,
	ref_i31 => RefI31, i31_get_s => I31GetS,
	// stack / misc
	drop => Drop, ret => Return, unreachable => Unreachable,
}
