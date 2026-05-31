// Per-function body emission. `FnEmitter` walks one function's IR `Block`,
// mapping each `VarId` to a wasm local and each `Rvalue` to GC/numeric
// instructions, and produces a `wasm_encoder::Function`. The arity-keyed
// uniform-boxed contract (see `lib.rs`) means a function's wasm signature is
// fixed by its arity; `var_reprs` says which locals are unboxed i64/f64/i32 and
// `Box`/`Unbox` mark the GC-ref boundaries the coercion pass already inserted.

use std::collections::{HashMap, HashSet};

use ir::{Atom, Block, Callee, Const, Repr, Rvalue, StmtKind};
use wasm_encoder::*;

use crate::Diagnostics;
use crate::runtime::{
	GlobalKind, GlobalSlot, Helper, Runtime, WIRE_FNV_OFFSET, host_sig, is_f64_unary_host,
	is_inline_builtin,
};
use crate::scan::{StrPool, block_has_pushdefer, builtin_var_tags, compute_nominal, ctor_var_tags};
use crate::types::{self, FuncTypes};
use crate::util::{EnumTable, binop_instr, repr_valtype, variant_display};

pub(crate) struct FnEmitter<'a> {
	f: &'a ir::Function,
	wasm_index: &'a HashMap<u32, u32>,
	host_index: &'a HashMap<String, u32>,
	gmap: &'a HashMap<u32, GlobalSlot>,
	runtime: Runtime,
	enums: &'a EnumTable,
	ftypes: &'a mut FuncTypes,
	var_tags: HashMap<u32, String>,
	/// VarId.0 -> variant tag, for vars bound to a `MakeVariantCtor`. Applying
	/// such a value (a `CallClosure` on it) builds the variant directly.
	var_ctors: HashMap<u32, (String, u32)>,
	/// VarId.0 -> record shape, for vars whose runtime value is a *nominal*
	/// `$shapeN` struct rather than the uniform `$record`. A record literal whose
	/// result is read as a record in this function (a `GetField` receiver or a
	/// record-pattern `Match` subject) is built nominal so the read is a constant-
	/// index `struct.get`; everywhere else it flows uniform. A nominal value
	/// reaching any other (uniform-consuming) position is `lift`ed to `$record`
	/// inline by `atom`. See `compute_nominal`.
	nominal: HashMap<u32, ir::RecordShape>,
	/// Per-function nominal param shapes (from record-shape monomorphization),
	/// keyed by `FuncId.0`. A `Some(S)` entry means that callee param is a nominal
	/// `$shapeN`, so an arg flowing into it is passed raw (not `lift`ed) and a
	/// `MakeRecord` arg is built nominal. This function's own params are seeded
	/// from its entry (via `compute_nominal`).
	param_shapes: &'a HashMap<u32, Vec<Option<ir::RecordShape>>>,
	strpool: &'a StrPool,
	diags: &'a mut Diagnostics,
	/// VarId.0 -> wasm local index. Wasm local 0 is the implicit closure env.
	locals: Vec<u32>,
	/// Local types for the locals past the wasm params, in declaration order.
	local_types: Vec<ValType>,
	/// Next free wasm local index (params occupy `0..=arity`).
	next_local: u32,
	/// Current control-flow nesting depth, for relative `br` targets.
	depth: u32,
	/// Enclosing loops as (continue-target level, break-target level).
	loop_stack: Vec<(u32, u32)>,
	/// For a function containing `defer`: the local holding the live cleanup list
	/// (a `$list` of zero-arg thunks, kept last-pushed-first). `None` for a
	/// defer-free function, which pays nothing. Each `PushDefer` prepends; each
	/// `Return` runs the list via `__run_defers` before returning.
	defers_local: Option<u32>,
	body: Vec<Instruction<'static>>,
}

impl<'a> FnEmitter<'a> {
	#[allow(clippy::too_many_arguments)]
	pub(crate) fn new(
		f: &'a ir::Function,
		fid: u32,
		wasm_index: &'a HashMap<u32, u32>,
		host_index: &'a HashMap<String, u32>,
		builtin_g: &HashMap<u32, String>,
		gmap: &'a HashMap<u32, GlobalSlot>,
		runtime: &Runtime,
		strpool: &'a StrPool,
		enums: &'a EnumTable,
		ftypes: &'a mut FuncTypes,
		param_shapes: &'a HashMap<u32, Vec<Option<ir::RecordShape>>>,
		extra_params: u32,
		diags: &'a mut Diagnostics,
	) -> Self {
		let var_tags = builtin_var_tags(&f.body, builtin_g);
		let var_ctors = ctor_var_tags(&f.body);
		let nominal = compute_nominal(f, fid, param_shapes);
		let n = f.var_reprs.len().max(f.params.len() + f.captures.len());
		let mut locals = vec![u32::MAX; n];
		// Wasm params: local 0 = env (closure ref/null), then the source params,
		// then any phantom params (the `fun { }` unit arg, mapped to no VarId).
		for (i, p) in f.params.iter().enumerate() {
			locals[p.0 as usize] = (i + 1) as u32;
		}
		let mut local_types = Vec::new();
		let mut next = (f.params.len() + 1) as u32 + extra_params;
		// Captures get locals too; loaded from the env in the prologue.
		for c in &f.captures {
			locals[c.0 as usize] = next;
			next += 1;
			local_types.push(types::value_ref());
		}
		// Every other var gets a fresh local, typed by its repr.
		for v in 0..n {
			if locals[v] == u32::MAX {
				locals[v] = next;
				next += 1;
				let repr = f.var_reprs.get(v).copied().unwrap_or(Repr::Boxed);
				local_types.push(repr_valtype(repr));
			}
		}
		Self {
			f,
			wasm_index,
			host_index,
			gmap,
			runtime: *runtime,
			enums,
			ftypes,
			var_tags,
			var_ctors,
			nominal,
			param_shapes,
			strpool,
			diags,
			locals,
			local_types,
			next_local: next,
			depth: 0,
			loop_stack: Vec::new(),
			defers_local: None,
			body: Vec::new(),
		}
	}

	pub(crate) fn emit(&mut self) -> Function {
		// Prologue: copy each captured value out of the env (`$closure` captures
		// array) into its local, so capture vars read like any other local.
		let caps: Vec<u32> = self.f.captures.iter().map(|c| c.0).collect();
		for (i, c) in caps.into_iter().enumerate() {
			let dst = self.local(c);
			self.ins(Instruction::LocalGet(0));
			self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
				types::T_CLOSURE,
			)));
			self.ins(Instruction::StructGet {
				struct_type_index: types::T_CLOSURE,
				field_index: 2,
			});
			self.ins(Instruction::I32Const(i as i32));
			self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			self.ins(Instruction::LocalSet(dst));
		}
		// A `defer`-bearing function threads a live cleanup list through a local,
		// started empty here; each `defer` prepends a thunk and each `Return` runs
		// the list LIFO. Defer-free functions allocate nothing.
		if block_has_pushdefer(&self.f.body) {
			let dl = self.fresh_local(types::value_ref());
			self.defers_local = Some(dl);
			self.ins(Instruction::I32Const(types::TAG_LIST));
			self.ins(Instruction::ArrayNewFixed {
				array_type_index: types::T_VALARRAY,
				array_size: 0,
			});
			self.ins(Instruction::StructNew(types::T_LIST));
			self.ins(Instruction::LocalSet(dl));
		}
		let body = self.f.body.clone();
		self.block(&body);
		let mut func = Function::new_with_locals_types(self.local_types.iter().copied());
		for ins in &self.body {
			func.instruction(ins);
		}
		func.instruction(&Instruction::End);
		func
	}

	/// Allocate a fresh wasm local of the given type, returning its index.
	fn fresh_local(&mut self, ty: ValType) -> u32 {
		let idx = self.next_local;
		self.next_local += 1;
		self.local_types.push(ty);
		idx
	}

	fn block(&mut self, b: &Block) {
		for s in &b.0 {
			self.stmt(&s.kind);
		}
	}

	fn stmt(&mut self, k: &StmtKind) {
		match k {
			StmtKind::Let(v, rv) => {
				// A record producer bound to a nominal var builds a `$shapeN` struct
				// (constant-index reads); otherwise the rvalue emits its uniform form.
				match rv {
					Rvalue::MakeRecord(fields) if self.nominal.contains_key(&v.0) => {
						let shape = self.nominal[&v.0].clone();
						self.make_record_nominal(&shape, fields);
					}
					Rvalue::RecordUpdate { base, fields } if self.nominal.contains_key(&v.0) => {
						let shape = self.nominal[&v.0].clone();
						self.record_update_nominal(&shape, base, fields);
					}
					_ => self.rvalue(rv),
				}
				self.ins(Instruction::LocalSet(self.local(v.0)));
			}
			StmtKind::Discard(rv) => {
				self.rvalue(rv);
				self.ins(Instruction::Drop);
			}
			StmtKind::Return(a) => {
				// Run scheduled `defer` cleanups (LIFO) before returning — matching
				// the VM, which runs the frame's cleanup stack at `Return`. The return
				// atom is side-effect-free (a var/const), so order vs. cleanups is
				// immaterial. `__run_defers` returns a `nothing` we drop.
				if let Some(dl) = self.defers_local {
					let run = self.runtime.idx(Helper::RunDefers).expect("run_defers");
					self.ins(Instruction::LocalGet(dl));
					self.ins(Instruction::Call(run));
					self.ins(Instruction::Drop);
				}
				self.atom(a);
				self.ins(Instruction::Return);
			}
			StmtKind::PushDefer(a) => {
				// Prepend the cleanup thunk onto the live `defers` list:
				// `defers = $list[__arrconcat([thunk], defers.elems)]`. Prepending
				// keeps the list last-pushed-first so `__run_defers` walks it LIFO.
				let Some(dl) = self.defers_local else {
					self.diags.push("PushDefer without a defers local");
					return;
				};
				let concat = self.runtime.idx(Helper::ArrConcat).expect("arrconcat");
				self.ins(Instruction::I32Const(types::TAG_LIST));
				// singleton `$valarray` [thunk].
				self.atom(a);
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: 1,
				});
				// defers.elems.
				self.ins(Instruction::LocalGet(dl));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
				self.ins(Instruction::Call(concat));
				self.ins(Instruction::StructNew(types::T_LIST));
				self.ins(Instruction::LocalSet(dl));
			}
			StmtKind::If(cond, t, e) => {
				self.atom(cond);
				self.ins(Instruction::If(wasm_encoder::BlockType::Empty));
				self.depth += 1;
				self.block(t);
				self.ins(Instruction::Else);
				self.block(e);
				self.ins(Instruction::End);
				self.depth -= 1;
			}
			StmtKind::Loop(b) => {
				let break_level = self.open_block();
				let cont_level = self.depth;
				self.ins(Instruction::Loop(wasm_encoder::BlockType::Empty));
				self.depth += 1;
				self.loop_stack.push((cont_level, break_level));
				self.block(b);
				// Back-edge: re-iterate the loop (exited via `Break`).
				self.ins(Instruction::Br(self.br_to(cont_level)));
				self.loop_stack.pop();
				self.ins(Instruction::End);
				self.depth -= 1;
				self.close_block();
			}
			StmtKind::Break => match self.loop_stack.last() {
				Some(&(_, brk)) => self.ins(Instruction::Br(self.br_to(brk))),
				None => self.diags.push("Break outside loop"),
			},
			StmtKind::Continue => match self.loop_stack.last() {
				Some(&(cont, _)) => self.ins(Instruction::Br(self.br_to(cont))),
				None => self.diags.push("Continue outside loop"),
			},
			StmtKind::Match { subject, arms } => self.match_stmt(subject, arms),
			other => self.diags.push(format!("unsupported stmt: {other:?}")),
		}
	}

	fn open_block(&mut self) -> u32 {
		let lvl = self.depth;
		self.ins(Instruction::Block(wasm_encoder::BlockType::Empty));
		self.depth += 1;
		lvl
	}

	fn close_block(&mut self) {
		self.ins(Instruction::End);
		self.depth -= 1;
	}

	/// The relative `br` immediate that targets the construct opened at `level`.
	fn br_to(&self, level: u32) -> u32 {
		self.depth - level - 1
	}

	/// Lower a pattern `Match`: evaluate the subject once, then try each arm in a
	/// nested block — a pattern mismatch `br`s past that arm to the next; a match
	/// runs the body and `br`s to the end (skipping later arms). No value is left
	/// on the stack (arms set locals or `Return`); the join, if any, is a local.
	fn match_stmt(&mut self, subject: &Atom, arms: &[ir::MatchArm]) {
		// A nominal-record subject keeps its `$shapeN` (pushed raw) so record-pattern
		// arms read fields by constant `struct.get`; a uniform subject scans by name.
		let subj_shape = match subject {
			Atom::Var(v) => self.nominal.get(&v.0).cloned(),
			_ => None,
		};
		let subj = self.fresh_local(types::value_ref());
		self.atom_raw(subject);
		self.ins(Instruction::LocalSet(subj));
		let end_level = self.open_block();
		for arm in arms {
			let arm_level = self.open_block();
			self.test_pattern(&arm.pattern, subj, subj_shape.as_ref(), arm_level);
			self.block(&arm.body);
			self.ins(Instruction::Br(self.br_to(end_level)));
			self.close_block();
		}
		self.close_block();
	}

	/// Test `pat` against the value in local `subj`. On mismatch, `br` to the
	/// block opened at `fail_level`. On match, bind any sub-vars. `subj_shape` is
	/// `Some` when `subj` holds a *nominal* `$shapeN` record (only the top-level
	/// `Match` subject is nominal; nested record fields are uniform), letting a
	/// record pattern read fields by constant `struct.get` instead of a name-scan.
	fn test_pattern(
		&mut self,
		pat: &ir::Pattern,
		subj: u32,
		subj_shape: Option<&ir::RecordShape>,
		fail_level: u32,
	) {
		use ir::Pattern::*;
		match pat {
			Wildcard => {}
			Bind(v) => {
				let dst = self.local(v.0);
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::LocalSet(dst));
			}
			Literal(c) => self.test_literal(c, subj, fail_level),
			Variant { variant, fields } => self.test_variant(variant, fields, subj, fail_level),
			Tuple(elems) => {
				// A tuple's arity is fixed by its type — no tag/length check.
				for (i, sub) in elems.iter().enumerate() {
					self.bind_at(sub, subj, types::T_TUPLE, 1, i, fail_level);
				}
			}
			List { items, rest } => {
				// Length: exact (== items) when no rest, else at-least (>= items).
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
				self.ins(Instruction::ArrayLen);
				self.ins(Instruction::I32Const(items.len() as i32));
				if rest.is_some() {
					self.ins(Instruction::I32LtS); // len < items -> fail
				} else {
					self.ins(Instruction::I32Ne); // len != items -> fail
				}
				self.ins(Instruction::BrIf(self.br_to(fail_level)));
				for (i, sub) in items.iter().enumerate() {
					self.bind_at(sub, subj, types::T_LIST, 1, i, fail_level);
				}
				if let Some(ir::ListRest::Bind(v)) = rest {
					// rest = __list_tail(list, items.len()).
					let tail = self.runtime.idx(Helper::ListTail).expect("list_tail");
					let dst = self.local(v.0);
					self.ins(Instruction::LocalGet(subj));
					self.ins(Instruction::I32Const(types::TAG_INT));
					self.ins(Instruction::I64Const(items.len() as i64));
					self.ins(Instruction::StructNew(types::T_INT));
					self.ins(Instruction::Call(tail));
					self.ins(Instruction::LocalSet(dst));
				}
			}
			Record {
				fields,
				rest,
				shape,
			} => {
				// Nominal subject: read each bound field by constant `struct.get`. The
				// `$shapeN` has exactly its shape's fields, so an `Exact` rest needs no
				// length check, and `...rest` builds the uniform `$record` of the
				// leftover fields (closing the WASM gap on the nominal path).
				if let Some(sshape) = subj_shape {
					let st = self.ftypes.intern_shape(&sshape.fields).type_idx;
					if let ir::RecordRest::Bind(v) = rest {
						let matched: HashSet<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
						let rest_fields: Vec<&String> = sshape
							.fields
							.iter()
							.filter(|n| !matched.contains(n.as_str()))
							.collect();
						let dst = self.local(v.0);
						self.ins(Instruction::I32Const(types::TAG_RECORD));
						for n in &rest_fields {
							self.string_const(n);
						}
						self.ins(Instruction::ArrayNewFixed {
							array_type_index: types::T_VALARRAY,
							array_size: rest_fields.len() as u32,
						});
						for n in &rest_fields {
							let slot = sshape.slot_of(n).unwrap();
							self.nominal_field(subj, st, slot);
						}
						self.ins(Instruction::ArrayNewFixed {
							array_type_index: types::T_VALARRAY,
							array_size: rest_fields.len() as u32,
						});
						self.ins(Instruction::StructNew(types::T_RECORD));
						self.ins(Instruction::LocalSet(dst));
					}
					for (name, sub) in fields {
						if matches!(sub, ir::Pattern::Wildcard) {
							continue;
						}
						let slot = sshape.slot_of(name).expect("nominal pattern field present");
						let tmp = self.fresh_local(types::value_ref());
						self.nominal_field(subj, st, slot);
						self.ins(Instruction::LocalSet(tmp));
						self.test_pattern(sub, tmp, None, fail_level);
					}
					return;
				}
				// Uniform subject: name-scan via `__getfield`.
				if let ir::RecordRest::Exact = rest {
					self.ins(Instruction::LocalGet(subj));
					self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
						types::T_RECORD,
					)));
					self.ins(Instruction::StructGet {
						struct_type_index: types::T_RECORD,
						field_index: 1,
					});
					self.ins(Instruction::ArrayLen);
					self.ins(Instruction::I32Const(fields.len() as i32));
					self.ins(Instruction::I32Ne);
					self.ins(Instruction::BrIf(self.br_to(fail_level)));
				}
				if let ir::RecordRest::Bind(v) = rest {
					// rest = __record_rest(subj, [matched names]) — the uniform `$record`
					// of the leftover fields, filtered by name at runtime.
					let Some(rr) = self.runtime.idx(Helper::RecordRest) else {
						self
							.diags
							.push("RecordRest used but __record_rest not emitted");
						return;
					};
					let dst = self.local(v.0);
					self.ins(Instruction::LocalGet(subj));
					// Build the excluded `$list` of matched field-name strings.
					self.ins(Instruction::I32Const(types::TAG_LIST));
					for (name, _) in fields {
						self.string_const(name);
					}
					self.ins(Instruction::ArrayNewFixed {
						array_type_index: types::T_VALARRAY,
						array_size: fields.len() as u32,
					});
					self.ins(Instruction::StructNew(types::T_LIST));
					self.ins(Instruction::Call(rr));
					self.ins(Instruction::LocalSet(dst));
				}
				let getfield = self.runtime.idx(Helper::GetField).expect("getfield");
				for (name, sub) in fields {
					match sub {
						ir::Pattern::Wildcard => {}
						_ => {
							// Step 2.0 debug cross-check (see the `GetField` rvalue path).
							self.debug_record_slot_guard(subj, name, shape);
							let tmp = self.fresh_local(types::value_ref());
							self.ins(Instruction::LocalGet(subj));
							self.string_const(name);
							self.ins(Instruction::Call(getfield));
							self.ins(Instruction::LocalSet(tmp));
							self.test_pattern(sub, tmp, None, fail_level);
						}
					}
				}
			}
		}
	}

	/// Match sub-pattern `sub` against element `i` of `subj` (a struct of type
	/// `sty` whose field `field` is the `$valarray`). Binds/recurses; on mismatch
	/// `br`s to `fail_level`.
	fn bind_at(&mut self, sub: &ir::Pattern, subj: u32, sty: u32, field: u32, i: usize, fail: u32) {
		match sub {
			ir::Pattern::Wildcard => {}
			ir::Pattern::Bind(v) => {
				let dst = self.local(v.0);
				self.get_elem(subj, sty, field, i);
				self.ins(Instruction::LocalSet(dst));
			}
			other => {
				let tmp = self.fresh_local(types::value_ref());
				self.get_elem(subj, sty, field, i);
				self.ins(Instruction::LocalSet(tmp));
				self.test_pattern(other, tmp, None, fail);
			}
		}
	}

	/// Push the inline field at `slot` of the nominal record in local `subj`
	/// (struct type `st`): cast to `$shapeN`, then `struct.get` field `2 + slot`
	/// (slots 0/1 are the tag/shape_id).
	fn nominal_field(&mut self, subj: u32, st: u32, slot: usize) {
		self.ins(Instruction::LocalGet(subj));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(st)));
		self.ins(Instruction::StructGet {
			struct_type_index: st,
			field_index: 2 + slot as u32,
		});
	}

	/// Push element `i` of the `$valarray` in field `field` of struct `subj:sty`.
	fn get_elem(&mut self, subj: u32, sty: u32, field: u32, i: usize) {
		self.ins(Instruction::LocalGet(subj));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(sty)));
		self.ins(Instruction::StructGet {
			struct_type_index: sty,
			field_index: field,
		});
		self.ins(Instruction::I32Const(i as i32));
		self.ins(Instruction::ArrayGet(types::T_VALARRAY));
	}

	/// Step 2.0 debug cross-check: assert the statically-resolved slot for `name`
	/// within `shape` matches the runtime record's layout. Reads `names[slot]` off
	/// the record in local `rec` and traps (`unreachable`) unless it equals the
	/// constant field name `name`. Stack-neutral, and emitted only when a closed
	/// shape was threaded *and* this is a debug build — release builds are
	/// byte-for-byte unchanged. The real field read still goes through the
	/// name-scan `__getfield`; this only validates that lowering threaded a slot
	/// consistent with the scan it currently shadows (the 2.1 representation flip
	/// will make the slot load-bearing).
	fn debug_record_slot_guard(&mut self, rec: u32, name: &str, shape: &Option<ir::RecordShape>) {
		if !cfg!(debug_assertions) {
			return;
		}
		let Some(shape) = shape else { return };
		let Some(slot) = shape.slot_of(name) else {
			return;
		};
		let Some(eq) = self.runtime.idx(Helper::Eq) else {
			return;
		};
		self.ins(Instruction::LocalGet(rec));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_RECORD,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_RECORD,
			field_index: 1,
		});
		self.ins(Instruction::I32Const(slot as i32));
		self.ins(Instruction::ArrayGet(types::T_VALARRAY));
		self.string_const(name);
		self.ins(Instruction::Call(eq));
		self.ins(Instruction::I32Eqz);
		self.ins(Instruction::If(wasm_encoder::BlockType::Empty));
		self.ins(Instruction::Unreachable);
		self.ins(Instruction::End);
	}

	fn test_literal(&mut self, c: &Const, subj: u32, fail_level: u32) {
		let br = self.br_to(fail_level);
		match c {
			Const::Bool(b) => {
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_BOOL,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_BOOL,
					field_index: 1,
				});
				self.ins(Instruction::I32Const(*b as i32));
				self.ins(Instruction::I32Ne);
				self.ins(Instruction::BrIf(br));
			}
			Const::Int(n) => {
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::I64Const(*n));
				self.ins(Instruction::I64Ne);
				self.ins(Instruction::BrIf(br));
			}
			Const::Float(x) => {
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_FLOAT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_FLOAT,
					field_index: 1,
				});
				self.ins(Instruction::F64Const((*x).into()));
				self.ins(Instruction::F64Ne);
				self.ins(Instruction::BrIf(br));
			}
			Const::Duration(n) => {
				// A `duration` reuses the `$int` shape (`{tag, i64}`); match it on
				// the nanosecond count, like an int literal.
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::I64Const(*n));
				self.ins(Instruction::I64Ne);
				self.ins(Instruction::BrIf(br));
			}
			Const::Str(_) | Const::Bytes(_) => {
				// Compare the (boxed) subject against the literal via structural
				// `__eq`; branch to the fail level when they differ.
				let Some(eq) = self.runtime.idx(Helper::Eq) else {
					self
						.diags
						.push("string/bytes pattern used but __eq not emitted");
					return;
				};
				self.ins(Instruction::LocalGet(subj));
				self.constant(c);
				self.ins(Instruction::Call(eq));
				self.ins(Instruction::I32Eqz); // not equal -> fail
				self.ins(Instruction::BrIf(br));
			}
			other => self
				.diags
				.push(format!("unsupported literal pattern: {other:?}")),
		}
	}

	fn test_variant(&mut self, name: &str, fields: &[ir::Pattern], subj: u32, fail_level: u32) {
		let Some(tag) = self.variant_tag(name) else {
			self.diags.push(format!("cannot resolve variant `{name}`"));
			return;
		};
		// Tag check.
		self.ins(Instruction::LocalGet(subj));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_VARIANT,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_VARIANT,
			field_index: 1,
		});
		self.ins(Instruction::I32Const(tag as i32));
		self.ins(Instruction::I32Ne);
		self.ins(Instruction::BrIf(self.br_to(fail_level)));
		// Bind / recurse on payload fields (variant payload is field 3).
		for (i, field) in fields.iter().enumerate() {
			self.bind_at(field, subj, types::T_VARIANT, 3, i, fail_level);
		}
	}

	/// Resolve a variant name to its within-enum tag. Sound when the name is
	/// unique across enums, or all its occurrences share a tag (the within-match
	/// enum is fixed by the type system, so a same-tag collision is harmless).
	fn variant_tag(&self, name: &str) -> Option<u32> {
		let mut found: Option<u32> = None;
		for variants in self.enums.values() {
			if let Some(i) = variants.iter().position(|(n, _)| n == name) {
				match found {
					None => found = Some(i as u32),
					Some(t) if t == i as u32 => {}
					Some(_) => return None, // ambiguous: differing tags
				}
			}
		}
		found
	}

	fn rvalue(&mut self, rv: &Rvalue) {
		match rv {
			Rvalue::Use(a) => self.atom(a),
			Rvalue::Bin(op @ (ir::BinOp::Eq | ir::BinOp::Ne), a, b) => {
				let Some(eq) = self.runtime.idx(Helper::Eq) else {
					self.diags.push("Eq/Ne used but __eq not emitted");
					return;
				};
				self.atom(a);
				self.atom(b);
				self.ins(Instruction::Call(eq));
				if matches!(op, ir::BinOp::Ne) {
					self.ins(Instruction::I32Eqz);
				}
			}
			Rvalue::Bin(ir::BinOp::Concat, a, b) => {
				// `++`: concatenate two strings' byte arrays, rewrap as `$str`.
				let Some(bc) = self.runtime.idx(Helper::BytesConcat) else {
					self.diags.push("Concat used but __bytesconcat not emitted");
					return;
				};
				self.str_bytes(a);
				self.str_bytes(b);
				self.ins(Instruction::Call(bc));
				let tmp = self.fresh_local(types::bytes_ref());
				self.ins(Instruction::LocalSet(tmp));
				self.ins(Instruction::I32Const(types::TAG_STR));
				self.ins(Instruction::LocalGet(tmp));
				self.ins(Instruction::StructNew(types::T_STR));
			}
			Rvalue::Bin(ir::BinOp::RemFloat, a, b) => {
				// f64 has no remainder opcode; compute `a - trunc(a/b)*b`, matching
				// the VM's `a % b` (Rust/IEEE `fmod`) for normal-magnitude operands.
				let la = self.fresh_local(ValType::F64);
				let lb = self.fresh_local(ValType::F64);
				self.atom(a);
				self.atom(b);
				self.ins(Instruction::LocalSet(lb));
				self.ins(Instruction::LocalSet(la));
				self.ins(Instruction::LocalGet(la));
				self.ins(Instruction::LocalGet(la));
				self.ins(Instruction::LocalGet(lb));
				self.ins(Instruction::F64Div);
				self.ins(Instruction::F64Trunc);
				self.ins(Instruction::LocalGet(lb));
				self.ins(Instruction::F64Mul);
				self.ins(Instruction::F64Sub);
			}
			Rvalue::Bin(op, a, b) => {
				self.atom(a);
				self.atom(b);
				match binop_instr(*op) {
					Some(ins) => self.ins(ins),
					None => self.diags.push(format!("unsupported binop: {op:?}")),
				}
			}
			Rvalue::Interpolate(parts) => {
				// Parts are already strings (the analyzer inserts `to-string`); fold
				// their byte arrays with `__bytesconcat`, rewrap as `$str`.
				if parts.is_empty() {
					self.ins(Instruction::I32Const(types::TAG_STR));
					self.ins(Instruction::ArrayNewFixed {
						array_type_index: types::T_BYTES,
						array_size: 0,
					});
					self.ins(Instruction::StructNew(types::T_STR));
					return;
				}
				let Some(bc) = self.runtime.idx(Helper::BytesConcat) else {
					self
						.diags
						.push("Interpolate used but __bytesconcat not emitted");
					return;
				};
				for (i, part) in parts.iter().enumerate() {
					self.str_bytes(part);
					if i > 0 {
						self.ins(Instruction::Call(bc));
					}
				}
				let tmp = self.fresh_local(types::bytes_ref());
				self.ins(Instruction::LocalSet(tmp));
				self.ins(Instruction::I32Const(types::TAG_STR));
				self.ins(Instruction::LocalGet(tmp));
				self.ins(Instruction::StructNew(types::T_STR));
			}
			Rvalue::Not(a) => {
				// `!b` over an i32 boolean: b == 0.
				self.atom(a);
				self.ins(Instruction::I32Eqz);
			}
			Rvalue::Box(a) => {
				let repr = self.atom_repr(a);
				let (tag, ty) = match repr {
					Repr::I64 => (types::TAG_INT, types::T_INT),
					Repr::F64 => (types::TAG_FLOAT, types::T_FLOAT),
					Repr::I32 => (types::TAG_BOOL, types::T_BOOL),
					Repr::Boxed => {
						self.diags.push("Box of an already-boxed value");
						return;
					}
				};
				self.ins(Instruction::I32Const(tag));
				self.atom(a);
				self.ins(Instruction::StructNew(ty));
			}
			Rvalue::Unbox(a, repr) => {
				self.atom(a);
				let (ty, field) = match repr {
					Repr::I64 => (types::T_INT, 1),
					Repr::F64 => (types::T_FLOAT, 1),
					Repr::I32 => (types::T_BOOL, 1),
					Repr::Boxed => {
						self.diags.push("Unbox to Boxed");
						return;
					}
				};
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(ty)));
				self.ins(Instruction::StructGet {
					struct_type_index: ty,
					field_index: field,
				});
			}
			Rvalue::Call(Callee::Function(fid), args) => {
				let Some(&w) = self.wasm_index.get(&fid.0) else {
					self.diags.push(format!("call to unreachable fn {}", fid.0));
					self.push_nothing();
					return;
				};
				// A direct call targets a capture-free function: pass a null env.
				self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
				// An arg flowing into a nominal (monomorphized) callee param is passed
				// raw (the `$shapeN`); other args are lifted to uniform by `atom`.
				let callee_shapes = self.param_shapes.get(&fid.0);
				for (i, a) in args.iter().enumerate() {
					let nominal_param = callee_shapes
						.and_then(|s| s.get(i))
						.map_or(false, |s| s.is_some());
					if nominal_param {
						self.atom_raw(a);
					} else {
						self.atom(a);
					}
				}
				self.ins(Instruction::Call(w));
			}
			Rvalue::CallClosure(callee, args) => self.call_value(callee, args, false),
			Rvalue::TailCall(callee, args) => {
				// A tail call would `return_call` past the trailing `Return`, skipping
				// any `defer` cleanups — so in a defer-bearing function, downgrade it
				// to an ordinary call and let the `Return` run the cleanups (mirroring
				// the VM, which suppresses TCO while a frame has pending cleanups).
				let tail = self.defers_local.is_none();
				self.call_value(callee, args, tail);
			}
			Rvalue::MakeClosure(fid, caps) => {
				let Some(&w) = self.wasm_index.get(&fid.0) else {
					self
						.diags
						.push(format!("closure over unreachable fn {}", fid.0));
					self.push_nothing();
					return;
				};
				self.ins(Instruction::I32Const(types::TAG_CLOSURE));
				self.ins(Instruction::I32Const(w as i32));
				for a in caps {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: caps.len() as u32,
				});
				self.ins(Instruction::StructNew(types::T_CLOSURE));
			}
			Rvalue::MakeVariant {
				enum_name,
				tag,
				payload,
			} => {
				self.ins(Instruction::I32Const(types::TAG_VARIANT));
				self.ins(Instruction::I32Const(*tag as i32));
				self.string_const(&variant_display(enum_name, *tag, self.enums));
				for a in payload {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: payload.len() as u32,
				});
				self.ins(Instruction::StructNew(types::T_VARIANT));
			}
			Rvalue::MakeVariantCtor { tag, enum_name } => {
				let arity = self.variant_arity(enum_name, *tag);
				self.ins(Instruction::I32Const(types::TAG_CTOR));
				self.ins(Instruction::I32Const(*tag as i32));
				self.ins(Instruction::I32Const(arity as i32));
				self.ins(Instruction::StructNew(types::T_CTOR));
			}
			Rvalue::MakeTuple(elems) => {
				self.ins(Instruction::I32Const(types::TAG_TUPLE));
				self.elems_array(elems);
				self.ins(Instruction::StructNew(types::T_TUPLE));
			}
			Rvalue::MakeList(items) => self.make_list(items),
			Rvalue::MakeRecord(fields) => {
				// Sort by field name for a canonical layout; names + values parallel.
				let mut sorted: Vec<(&String, &Atom)> = fields.iter().map(|(n, a)| (n, a)).collect();
				sorted.sort_by(|a, b| a.0.cmp(b.0));
				self.ins(Instruction::I32Const(types::TAG_RECORD));
				for (n, _) in &sorted {
					self.string_const(n);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: sorted.len() as u32,
				});
				for (_, a) in &sorted {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: sorted.len() as u32,
				});
				self.ins(Instruction::StructNew(types::T_RECORD));
			}
			Rvalue::GetField(r, name, shape) => {
				// Nominal fast path: a record built nominal in this function is a
				// `$shapeN` struct, so the field read is a constant-index `struct.get`
				// (slot 0/1 are the tag/shape_id; fields start at 2, in shape order).
				if let Atom::Var(v) = r {
					if let Some(rshape) = self.nominal.get(&v.0).cloned() {
						if let Some(slot) = rshape.slot_of(name) {
							let st = self.ftypes.intern_shape(&rshape.fields).type_idx;
							self.atom_raw(r);
							self.ins(Instruction::RefCastNonNull(HeapType::Concrete(st)));
							self.ins(Instruction::StructGet {
								struct_type_index: st,
								field_index: 2 + slot as u32,
							});
							return;
						}
					}
				}
				// Uniform path: name-scan via `__getfield`. The receiver is the uniform
				// `$record` here (`atom` lifts a nominal arg, though a nominal receiver
				// already took the fast path above).
				let Some(getfield) = self.runtime.idx(Helper::GetField) else {
					self.diags.push("GetField used but __getfield not emitted");
					return;
				};
				// Step 2.0 debug cross-check: assert the statically-resolved slot agrees
				// with the runtime name-scan layout (only meaningful on the uniform
				// `$record`, which carries the `names` array).
				if let Atom::Var(v) = r {
					let rec_local = self.local(v.0);
					self.debug_record_slot_guard(rec_local, name, shape);
				}
				self.atom(r);
				self.string_const(name);
				self.ins(Instruction::Call(getfield));
			}
			Rvalue::RecordUpdate { base, fields } => {
				let Some(update) = self.runtime.idx(Helper::RecordUpdate) else {
					self
						.diags
						.push("RecordUpdate used but __record_update not emitted");
					return;
				};
				// __record_update(base, name, value) applied once per override.
				self.atom(base);
				for (n, a) in fields {
					self.string_const(n);
					self.atom(a);
					self.ins(Instruction::Call(update));
				}
			}
			Rvalue::MakeDict(methods) => {
				self.ins(Instruction::I32Const(types::TAG_METHODDICT));
				for a in methods {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: methods.len() as u32,
				});
				self.ins(Instruction::StructNew(types::T_METHODDICT));
			}
			Rvalue::GetDictMethod(dict, idx) => {
				self.atom(dict);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_METHODDICT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_METHODDICT,
					field_index: 1,
				});
				self.ins(Instruction::I32Const(*idx as i32));
				self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			}
			Rvalue::GetTag(a) => {
				self.atom(a);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_VARIANT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_VARIANT,
					field_index: 1,
				});
			}
			Rvalue::GetPayload(a, i) => {
				self.atom(a);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_VARIANT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_VARIANT,
					field_index: 3, // payload (after tag, vtag, name)
				});
				self.ins(Instruction::I32Const(*i as i32));
				self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			}
			Rvalue::GetElement(a, i) => {
				self.atom(a);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_TUPLE,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_TUPLE,
					field_index: 1, // elems array (after tag)
				});
				self.ins(Instruction::I32Const(*i as i32));
				self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			}
			Rvalue::GlobalRef(g) => {
				if let Some(slot) = self.gmap.get(&g.0).cloned() {
					// Lazy: build the value once, cache behind the init flag, then load.
					self.ins(Instruction::GlobalGet(slot.init_idx));
					self.ins(Instruction::I32Eqz);
					self.ins(Instruction::If(wasm_encoder::BlockType::Empty));
					self.depth += 1;
					match &slot.kind {
						GlobalKind::Thunk(thunk_wasm) => {
							self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE))); // env
							self.ins(Instruction::Call(*thunk_wasm));
						}
						GlobalKind::MethodDict(wrappers) => {
							// Build a $methoddict of capture-free wrapper closures.
							self.ins(Instruction::I32Const(types::TAG_METHODDICT));
							for &w in wrappers {
								self.ins(Instruction::I32Const(types::TAG_CLOSURE));
								self.ins(Instruction::I32Const(w as i32));
								self.ins(Instruction::ArrayNewFixed {
									array_type_index: types::T_VALARRAY,
									array_size: 0,
								});
								self.ins(Instruction::StructNew(types::T_CLOSURE));
							}
							self.ins(Instruction::ArrayNewFixed {
								array_type_index: types::T_VALARRAY,
								array_size: wrappers.len() as u32,
							});
							self.ins(Instruction::StructNew(types::T_METHODDICT));
						}
					}
					self.ins(Instruction::GlobalSet(slot.val_idx));
					self.ins(Instruction::I32Const(1));
					self.ins(Instruction::GlobalSet(slot.init_idx));
					self.ins(Instruction::End);
					self.depth -= 1;
					self.ins(Instruction::GlobalGet(slot.val_idx));
				} else {
					// A builtin-global reference used only as a call target: emit a null
					// placeholder (its only consumer is the call site, special-cased).
					self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
				}
			}
			other => self.diags.push(format!("unsupported rvalue: {other:?}")),
		}
	}

	/// Dispatch a `CallClosure`/`TailCall` by callee kind: host builtin, a partial
	/// variant constructor (build the variant), or a runtime closure.
	fn call_value(&mut self, callee: &Atom, args: &[Atom], tail: bool) {
		if let Some(tag) = self.callee_tag(callee) {
			self.host_call(&tag, args);
			return;
		}
		if let Atom::Var(v) = callee {
			if let Some((enum_name, tag)) = self.var_ctors.get(&v.0).cloned() {
				// Applying a constructor builds the variant directly.
				self.ins(Instruction::I32Const(types::TAG_VARIANT));
				self.ins(Instruction::I32Const(tag as i32));
				self.string_const(&variant_display(&enum_name, tag, self.enums));
				for a in args {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: args.len() as u32,
				});
				self.ins(Instruction::StructNew(types::T_VARIANT));
				return;
			}
		}
		self.closure_call(callee, args, tail);
	}

	/// `CallClosure`/`TailCall` on a runtime closure value: pass the closure as
	/// the env (param 0), then the args, then `call_indirect` through its stored
	/// `fn_index`.
	fn closure_call(&mut self, callee: &Atom, args: &[Atom], tail: bool) {
		let arity = args.len();
		// env = the closure value.
		self.atom(callee);
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_CLOSURE,
		)));
		for a in args {
			self.atom(a);
		}
		// fn_index from the closure.
		self.atom(callee);
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_CLOSURE,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_CLOSURE,
			field_index: 1,
		});
		let ty = self.ftypes.for_arity(arity);
		if tail {
			self.ins(Instruction::ReturnCallIndirect {
				type_index: ty,
				table_index: 0,
			});
		} else {
			self.ins(Instruction::CallIndirect {
				type_index: ty,
				table_index: 0,
			});
		}
	}

	fn variant_arity(&self, enum_name: &str, tag: u32) -> usize {
		self
			.enums
			.get(enum_name)
			.and_then(|vs| vs.get(tag as usize))
			.map(|(_, a)| *a)
			.unwrap_or(0)
	}

	/// The builtin tag a callee atom resolves to, if any.
	fn callee_tag(&self, callee: &Atom) -> Option<String> {
		if let Atom::Var(v) = callee {
			self.var_tags.get(&v.0).cloned()
		} else {
			None
		}
	}

	fn host_call(&mut self, tag: &str, args: &[Atom]) {
		// Pure-compute builtins emitted inline over the `$value` GC layout.
		if is_inline_builtin(tag) {
			self.inline_builtin(tag, args);
			return;
		}
		// Unary float math (log/exp/sin/cos): unbox the `$float`, call the raw
		// `(f64) -> f64` host import, rebox. Keeps the GC poking in wasm.
		if is_f64_unary_host(tag) {
			match self.host_index.get(tag).copied() {
				Some(idx) => {
					self.ins(Instruction::I32Const(types::TAG_FLOAT));
					self.atom(&args[0]);
					self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
						types::T_FLOAT,
					)));
					self.ins(Instruction::StructGet {
						struct_type_index: types::T_FLOAT,
						field_index: 1,
					});
					self.ins(Instruction::Call(idx));
					self.ins(Instruction::StructNew(types::T_FLOAT));
				}
				None => {
					self.diags.push(format!("`{tag}` host import not declared"));
					self.push_nothing();
				}
			}
			return;
		}
		// Higher-order builders: synthetic helpers (loop + closure call).
		if tag == "list-build" || tag == "list-collect" || tag == "bytes-build" {
			let helper = match tag {
				"list-build" => self.runtime.idx(Helper::ListBuild),
				"list-collect" => self.runtime.idx(Helper::ListCollect),
				_ => self.runtime.idx(Helper::BytesBuild),
			};
			match helper {
				Some(h) => {
					for a in args {
						self.atom(a);
					}
					self.ins(Instruction::Call(h));
				}
				None => {
					self.diags.push(format!("`{tag}` helper not emitted"));
					self.push_nothing();
				}
			}
			return;
		}
		// bytes.concat a b : a fresh `bytes` of a's bytes then b's, via __bytesconcat.
		if tag == "bytes-concat" {
			match self.runtime.idx(Helper::BytesConcat) {
				Some(bc) => {
					self.ins(Instruction::I32Const(types::TAG_BYTES));
					self.str_bytes(&args[0]);
					self.str_bytes(&args[1]);
					self.ins(Instruction::Call(bc));
					self.ins(Instruction::StructNew(types::T_STR));
				}
				None => {
					self
						.diags
						.push("bytes-concat needs __bytesconcat".to_string());
					self.push_nothing();
				}
			}
			return;
		}
		// dict scan/rebuild/closure ops → synthetic helpers. insert/lookup/remove
		// receive a hash method-dict as `args[0]` (the `where (hash k)` evidence);
		// the wasm dict scans with `__eq` instead of hashing, so that arg is DROPPED
		// — we pass only the dict + key (+ value). map/filter take `[dict, f]`.
		if let Some((helper, call_args)) = match tag {
			"dict-insert" => Some((self.runtime.idx(Helper::DictInsert), &args[1..])),
			"dict-lookup" => Some((self.runtime.idx(Helper::DictLookup), &args[1..])),
			"dict-remove" => Some((self.runtime.idx(Helper::DictRemove), &args[1..])),
			"dict-map" => Some((self.runtime.idx(Helper::DictMap), &args[0..])),
			"dict-filter" => Some((self.runtime.idx(Helper::DictFilter), &args[0..])),
			_ => None,
		} {
			match helper {
				Some(h) => {
					for a in call_args {
						self.atom(a);
					}
					self.ins(Instruction::Call(h));
				}
				None => {
					self.diags.push(format!("`{tag}` helper not emitted"));
					self.push_nothing();
				}
			}
			return;
		}
		// `wire-fingerprint`: FNV-1a hash of the schema tree, boxed as `$int`.
		if tag == "wire-fingerprint" {
			match self.runtime.idx(Helper::WireFp) {
				Some(fp) => {
					self.ins(Instruction::I32Const(types::TAG_INT));
					self.ins(Instruction::I64Const(WIRE_FNV_OFFSET));
					self.atom(&args[0]);
					self.ins(Instruction::Call(fp));
					self.ins(Instruction::StructNew(types::T_INT));
					return;
				}
				None => {
					self
						.diags
						.push("wire-fingerprint needs __wire_fp".to_string());
					self.push_nothing();
					return;
				}
			}
		}
		// `wire-encode` (args `[schema, value]`): reset the codec globals, run the
		// recursive encoder into `g_buf`, then snapshot `g_buf[0..g_len]` into an
		// exact-size `$bytes`, wrapped `TAG_BYTES`.
		if tag == "wire-encode" {
			let Some(enc) = self.runtime.idx(Helper::WireEnc) else {
				self.diags.push("wire-encode needs __wire_enc".to_string());
				self.push_nothing();
				return;
			};
			let g = self.runtime.wireg;
			self.ins(Instruction::I32Const(64));
			self.ins(Instruction::ArrayNewDefault(types::T_BYTES));
			self.ins(Instruction::GlobalSet(g.buf));
			self.ins(Instruction::I32Const(0));
			self.ins(Instruction::GlobalSet(g.len));
			self.ins(Instruction::I32Const(0));
			self.ins(Instruction::GlobalSet(g.ctxlen));
			self.ins(Instruction::I32Const(8));
			self.ins(Instruction::ArrayNewDefault(types::T_VALARRAY));
			self.ins(Instruction::GlobalSet(g.ctx));
			self.atom(&args[0]);
			self.atom(&args[1]);
			self.ins(Instruction::Call(enc));
			let res = self.fresh_local(types::bytes_ref());
			self.ins(Instruction::GlobalGet(g.len));
			self.ins(Instruction::ArrayNewDefault(types::T_BYTES));
			self.ins(Instruction::LocalSet(res));
			self.ins(Instruction::LocalGet(res));
			self.ins(Instruction::I32Const(0));
			self.ins(Instruction::GlobalGet(g.buf));
			self.ins(Instruction::I32Const(0));
			self.ins(Instruction::GlobalGet(g.len));
			self.ins(Instruction::ArrayCopy {
				array_type_index_dst: types::T_BYTES,
				array_type_index_src: types::T_BYTES,
			});
			self.ins(Instruction::I32Const(types::TAG_BYTES));
			self.ins(Instruction::LocalGet(res));
			self.ins(Instruction::StructNew(types::T_STR));
			return;
		}
		// `wire-decode` (args `[schema, bytes]`): point the codec at the input,
		// reset cursor/error/registry, run the decoder, then wrap the value in
		// `ok`/`err` (the trailing-bytes check rides in `__wire_result`).
		if tag == "wire-decode" {
			let (Some(dec), Some(result)) = (
				self.runtime.idx(Helper::WireDec),
				self.runtime.idx(Helper::WireResult),
			) else {
				self.diags.push("wire-decode needs __wire_dec".to_string());
				self.push_nothing();
				return;
			};
			let g = self.runtime.wireg;
			self.atom(&args[1]);
			self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
				types::T_STR,
			)));
			self.ins(Instruction::StructGet {
				struct_type_index: types::T_STR,
				field_index: 1,
			});
			self.ins(Instruction::GlobalSet(g.input));
			self.ins(Instruction::I32Const(0));
			self.ins(Instruction::GlobalSet(g.pos));
			self.ins(Instruction::I32Const(0));
			self.ins(Instruction::GlobalSet(g.err));
			self.ins(Instruction::I64Const(0));
			self.ins(Instruction::GlobalSet(g.errval));
			self.ins(Instruction::I32Const(0));
			self.ins(Instruction::GlobalSet(g.ctxlen));
			self.ins(Instruction::I32Const(8));
			self.ins(Instruction::ArrayNewDefault(types::T_VALARRAY));
			self.ins(Instruction::GlobalSet(g.ctx));
			self.atom(&args[0]);
			self.ins(Instruction::Call(dec));
			self.ins(Instruction::Call(result));
			return;
		}
		// `to-string` is implemented in wasm (`__tostring`), not imported.
		if tag == "to-string" {
			if let (Some(ts), Some(a)) = (self.runtime.idx(Helper::ToString), args.first()) {
				self.atom(a);
				self.ins(Instruction::Call(ts));
				return;
			}
			self.diags.push("to-string used but __tostring not emitted");
			self.push_nothing();
			return;
		}
		let Some(&idx) = self.host_index.get(tag) else {
			self.diags.push(format!("unsupported host builtin `{tag}`"));
			self.push_nothing();
			return;
		};
		for a in args {
			self.atom(a);
		}
		self.ins(Instruction::Call(idx));
		// `print`/`debug` return nothing; the `Let` binding expects a value, so
		// materialize `nothing`.
		if !host_sig(tag).map(|s| s.returns_value).unwrap_or(true) {
			self.push_nothing();
		}
	}

	/// Emit a pure-compute builtin inline over the `$value` GC layout.
	/// Leaves one `$value` on the stack (the binding's rvalue).
	fn inline_builtin(&mut self, tag: &str, args: &[Atom]) {
		match tag {
			// list.get xs i : the i-th element. (`$int` index unboxed to i32.)
			"list-get" => {
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
				self.atom(&args[1]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::I32WrapI64);
				self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			}
			// list.set xs i v : overwrite the i-th slot in place; yields nothing.
			// The deliberate escape hatch from list immutability.
			"list-set" => {
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
				self.atom(&args[1]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::I32WrapI64);
				self.atom(&args[2]);
				self.ins(Instruction::ArraySet(types::T_VALARRAY));
				self.push_nothing();
			}
			// ref.new v : a fresh `$ref` cell holding `v`.
			"ref-new" => {
				self.ins(Instruction::I32Const(types::TAG_REF));
				self.atom(&args[0]);
				self.ins(Instruction::StructNew(types::T_REF));
			}
			// ref.get r : the cell's current value.
			"ref-get" => {
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_REF,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_REF,
					field_index: 1,
				});
			}
			// ref.set r v : overwrite the cell in place; yields nothing.
			"ref-set" => {
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_REF,
				)));
				self.atom(&args[1]);
				self.ins(Instruction::StructSet {
					struct_type_index: types::T_REF,
					field_index: 1,
				});
				self.push_nothing();
			}
			// ref.update r f : read once, apply the closure `f`, write back; yields
			// nothing. The closure call is emitted inline (env + 1 arg, then
			// `call_indirect` through its stored fn_index), with the cell struct kept
			// underneath for the final `struct.set`.
			"ref-update" => {
				let cell = self.fresh_local(types::ref_ref());
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_REF,
				)));
				self.ins(Instruction::LocalTee(cell)); // stack: [cell]
				// closure env = f.
				self.atom(&args[1]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_CLOSURE,
				)));
				// arg = the cell's current value.
				self.ins(Instruction::LocalGet(cell));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_REF,
					field_index: 1,
				});
				// fn_index from the closure, then call_indirect (arity 1).
				self.atom(&args[1]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_CLOSURE,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_CLOSURE,
					field_index: 1,
				});
				let ty = self.ftypes.for_arity(1);
				self.ins(Instruction::CallIndirect {
					type_index: ty,
					table_index: 0,
				}); // stack: [cell, new_value]
				self.ins(Instruction::StructSet {
					struct_type_index: types::T_REF,
					field_index: 1,
				});
				self.push_nothing();
			}
			// dict.empty () : a fresh `$dict` with no entries. (arg is the unit.)
			"dict-empty" => {
				self.ins(Instruction::I32Const(types::TAG_DICT));
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: 0,
				});
				self.ins(Instruction::StructNew(types::T_DICT));
			}
			// dict.size m : entry count, boxed as `$int`.
			"dict-size" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_DICT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_DICT,
					field_index: 1,
				});
				self.ins(Instruction::ArrayLen);
				self.ins(Instruction::I64ExtendI32U);
				self.ins(Instruction::StructNew(types::T_INT));
			}
			// dict.entries m : the (k,v) tuples as a `$list`. The dict's entries
			// array is already a `$valarray` of `$tuple`s — just retag it as a list
			// (shared; neither side mutates it in place).
			"dict-entries" => {
				self.ins(Instruction::I32Const(types::TAG_LIST));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_DICT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_DICT,
					field_index: 1,
				});
				self.ins(Instruction::StructNew(types::T_LIST));
			}
			// list.length xs : element count, boxed as `$int`.
			"list-length" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
				self.ins(Instruction::ArrayLen);
				self.ins(Instruction::I64ExtendI32U);
				self.ins(Instruction::StructNew(types::T_INT));
			}
			// bytes.get b i : the i-th byte (0-255) as `$int`. (`$bytes` is packed
			// i8, read unsigned.)
			"bytes-get" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_STR,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_STR,
					field_index: 1,
				});
				self.atom(&args[1]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::I32WrapI64);
				self.ins(Instruction::ArrayGetU(types::T_BYTES));
				self.ins(Instruction::I64ExtendI32U);
				self.ins(Instruction::StructNew(types::T_INT));
			}
			// bytes.length b : byte count, boxed as `$int`.
			"bytes-length" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_STR,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_STR,
					field_index: 1,
				});
				self.ins(Instruction::ArrayLen);
				self.ins(Instruction::I64ExtendI32U);
				self.ins(Instruction::StructNew(types::T_INT));
			}
			// bytes <-> string reinterpret: same `{tag, $bytes}` shape, just
			// rebuild the struct with the other tag (no copy, no validation).
			"bytes-as-string" | "string-to-bytes" => {
				let new_tag = if tag == "bytes-as-string" {
					types::TAG_STR
				} else {
					types::TAG_BYTES
				};
				self.ins(Instruction::I32Const(new_tag));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_STR,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_STR,
					field_index: 1,
				});
				self.ins(Instruction::StructNew(types::T_STR));
			}
			// math.sqrt f : unbox the f64, `f64.sqrt`, rebox. NaN for f < 0,
			// matching the IEEE-754 result the VM's `f64::sqrt` yields.
			"math-sqrt" => {
				self.ins(Instruction::I32Const(types::TAG_FLOAT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_FLOAT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_FLOAT,
					field_index: 1,
				});
				self.ins(Instruction::F64Sqrt);
				self.ins(Instruction::StructNew(types::T_FLOAT));
			}
			// math.to-int f : truncate toward zero into an i64. The *saturating*
			// trunc matches the VM's `f as i64` (NaN -> 0, ±inf / out-of-range
			// clamp to i64::MIN/MAX); plain `i64.trunc_f64_s` would trap instead.
			"math-to-int" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_FLOAT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_FLOAT,
					field_index: 1,
				});
				self.ins(Instruction::I64TruncSatF64S);
				self.ins(Instruction::StructNew(types::T_INT));
			}
			// math.to-float n : widen the i64 to f64.
			"math-to-float" => {
				self.ins(Instruction::I32Const(types::TAG_FLOAT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::F64ConvertI64S);
				self.ins(Instruction::StructNew(types::T_FLOAT));
			}
			// time.as-nanos d : a `duration`'s nanosecond count as an `int`. A
			// `duration` reuses the `$int` shape (`{tag, i64}`), tagged `TAG_DURATION`,
			// so this just reads the i64 and reboxes it `TAG_INT` (the other `as-*`
			// accessors are pure Pluma over this one).
			"time-duration-as-nanos" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_INT,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_INT,
					field_index: 1,
				});
				self.ins(Instruction::StructNew(types::T_INT));
			}
			// `instant` / `duration` box+unbox: both reuse the `$int` shape
			// (`{tag, i64}`). The `*-of-nanos` / `from-unix-nanos` direction reboxes
			// an `int` payload under the carrier tag; `to-unix-nanos` reads the i64
			// back out under `TAG_INT`. (`duration-as-nanos` above is the same retag.)
			"time-duration-of-nanos" => self.retag_int_box(&args[0], types::TAG_DURATION),
			"time-from-unix-nanos" => self.retag_int_box(&args[0], types::TAG_INSTANT),
			"time-to-unix-nanos" => self.retag_int_box(&args[0], types::TAG_INT),
			_ => {
				self
					.diags
					.push(format!("inline builtin `{tag}` not emitted"));
				self.push_nothing();
			}
		}
	}

	/// Retag an `$int`-shaped box (`{tag, i64}`) under `new_tag`: read the i64
	/// payload out and rebox it. The `duration` and `instant` carriers share the
	/// `$int` struct shape and differ only by tag, so box/unbox between them and a
	/// plain `int` is just this retag.
	fn retag_int_box(&mut self, arg: &Atom, new_tag: i32) {
		self.ins(Instruction::I32Const(new_tag));
		self.atom(arg);
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_INT,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_INT,
			field_index: 1,
		});
		self.ins(Instruction::StructNew(types::T_INT));
	}

	/// Push an atom as a uniform boxed `$value`. A var holding a *nominal* record
	/// (`$shapeN`) is `lift`ed to the uniform `$record` here, so every consumer that
	/// isn't a record read (a call arg, a container element, a `Return`, a stored
	/// field, a generic consumer) sees the self-describing representation it
	/// expects. Read sites (`GetField` receiver, `Match` subject) use `atom_raw`
	/// instead, keeping the `$shapeN` for a constant-index `struct.get`.
	fn atom(&mut self, a: &Atom) {
		if let Atom::Var(v) = a {
			if let Some(shape) = self.nominal.get(&v.0).cloned() {
				self.emit_lift(self.local(v.0), &shape);
				return;
			}
		}
		self.atom_raw(a);
	}

	/// Push an atom with no representation coercion — a bare `LocalGet`/constant.
	/// For a nominal-record var this leaves the `$shapeN` struct on the stack.
	fn atom_raw(&mut self, a: &Atom) {
		match a {
			Atom::Var(v) => self.ins(Instruction::LocalGet(self.local(v.0))),
			Atom::Const(c) => self.constant(c),
		}
	}

	/// `lift` a nominal record in local `rec` to the uniform `$record`: build the
	/// name-sorted `names` array (constant field-name strings) and a parallel
	/// `values` array read out of the struct's inline fields, then `struct.new
	/// $record`. Leaves one `(ref $record)` on the stack; reads nothing else.
	fn emit_lift(&mut self, rec: u32, shape: &ir::RecordShape) {
		let st = self.ftypes.intern_shape(&shape.fields).type_idx;
		let k = shape.fields.len() as u32;
		self.ins(Instruction::I32Const(types::TAG_RECORD));
		for name in &shape.fields {
			self.string_const(name);
		}
		self.ins(Instruction::ArrayNewFixed {
			array_type_index: types::T_VALARRAY,
			array_size: k,
		});
		for i in 0..k {
			self.ins(Instruction::LocalGet(rec));
			self.ins(Instruction::RefCastNonNull(HeapType::Concrete(st)));
			self.ins(Instruction::StructGet {
				struct_type_index: st,
				field_index: 2 + i,
			});
		}
		self.ins(Instruction::ArrayNewFixed {
			array_type_index: types::T_VALARRAY,
			array_size: k,
		});
		self.ins(Instruction::StructNew(types::T_RECORD));
	}

	/// Build a *nominal* record: a `$shapeN` struct `{ tag, shape_id, f0..fk }` with
	/// the field values inline in the shape's name-sorted order. Field values are
	/// pushed via `atom` (so a nested nominal record is stored as the uniform
	/// `$record`, keeping field reads uniform). The result is a `(ref $shapeN)`,
	/// storable in a `(ref null $value)` local (it's a `$value` subtype).
	fn make_record_nominal(&mut self, shape: &ir::RecordShape, fields: &[(String, Atom)]) {
		let st = self.ftypes.intern_shape(&shape.fields);
		let mut sorted: Vec<(&String, &Atom)> = fields.iter().map(|(n, a)| (n, a)).collect();
		sorted.sort_by(|a, b| a.0.cmp(b.0));
		self.ins(Instruction::I32Const(types::TAG_RECORD));
		self.ins(Instruction::I32Const(st.shape_id as i32));
		for (_, a) in &sorted {
			self.atom(a);
		}
		self.ins(Instruction::StructNew(st.type_idx));
	}

	/// Build a record-update `{ ...base, f: v }` on a *nominal* base of the same
	/// shape: a fresh `$shapeN` whose fields are the overrides where given, else
	/// `base`'s inline field at that slot — a `struct.new` copy, no array
	/// allocation or name-scan (the uniform `__record_update` path). The base is a
	/// nominal var (read raw); override values are stored uniform (via `atom`).
	fn record_update_nominal(
		&mut self,
		shape: &ir::RecordShape,
		base: &Atom,
		fields: &[(String, Atom)],
	) {
		let st = self.ftypes.intern_shape(&shape.fields);
		let base_local = match base {
			Atom::Var(v) => self.local(v.0),
			// A record base is always a var; fall back to the uniform path otherwise.
			Atom::Const(_) => {
				self.diags.push("record-update on a non-var nominal base");
				return;
			}
		};
		self.ins(Instruction::I32Const(types::TAG_RECORD));
		self.ins(Instruction::I32Const(st.shape_id as i32));
		for (i, name) in shape.fields.iter().enumerate() {
			match fields.iter().find(|(n, _)| n == name) {
				Some((_, a)) => self.atom(a),
				None => self.nominal_field(base_local, st.type_idx, i),
			}
		}
		self.ins(Instruction::StructNew(st.type_idx));
	}

	fn constant(&mut self, c: &Const) {
		match c {
			Const::Int(n) => self.ins(Instruction::I64Const(*n)),
			Const::Float(x) => self.ins(Instruction::F64Const((*x).into())),
			Const::Bool(b) => self.ins(Instruction::I32Const(*b as i32)),
			Const::Unit => self.push_nothing(),
			Const::Str(s) => self.string_const(s),
			Const::Duration(n) => {
				self.ins(Instruction::I32Const(types::TAG_DURATION));
				self.ins(Instruction::I64Const(*n));
				self.ins(Instruction::StructNew(types::T_INT));
			}
			Const::Bytes(b) => self.bytes_const(b),
		}
	}

	/// A `bytes` literal: the `$str`-shaped struct (`{tag, ref $bytes}`) tagged
	/// `TAG_BYTES`. Backing bytes come from the shared passive data segment.
	fn bytes_const(&mut self, b: &[u8]) {
		let Some(&(off, len)) = self.strpool.bytes_at.get(b) else {
			self
				.diags
				.push("bytes constant missing from pool".to_string());
			return;
		};
		self.ins(Instruction::I32Const(types::TAG_BYTES));
		self.ins(Instruction::I32Const(off as i32));
		self.ins(Instruction::I32Const(len as i32));
		self.ins(Instruction::ArrayNewData {
			array_type_index: types::T_BYTES,
			array_data_index: 0,
		});
		self.ins(Instruction::StructNew(types::T_STR));
	}

	fn push_nothing(&mut self) {
		self.ins(Instruction::I32Const(types::TAG_NOTHING));
		self.ins(Instruction::StructNew(types::T_VALUE));
	}

	/// Push a `$str` value for a string constant (from the shared data segment).
	fn string_const(&mut self, s: &str) {
		let Some(&(off, len)) = self.strpool.at.get(s) else {
			self
				.diags
				.push(format!("string constant {s:?} missing from pool"));
			return;
		};
		self.ins(Instruction::I32Const(types::TAG_STR));
		self.ins(Instruction::I32Const(off as i32));
		self.ins(Instruction::I32Const(len as i32));
		self.ins(Instruction::ArrayNewData {
			array_type_index: types::T_BYTES,
			array_data_index: 0,
		});
		self.ins(Instruction::StructNew(types::T_STR));
	}

	/// Push the `$bytes` backing array of a string-typed atom.
	fn str_bytes(&mut self, a: &Atom) {
		self.atom(a);
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_STR,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_STR,
			field_index: 1,
		});
	}

	/// Push a `$valarray` built from the given atoms (boxed).
	fn elems_array(&mut self, elems: &[Atom]) {
		for a in elems {
			self.atom(a);
		}
		self.ins(Instruction::ArrayNewFixed {
			array_type_index: types::T_VALARRAY,
			array_size: elems.len() as u32,
		});
	}

	/// `MakeList`: an element-only list builds a `$valarray` directly. A spread
	/// (`[a, ...xs, b]`) builds each segment's array — a fixed array for each run
	/// of plain elements, a list's element array for each `...spread` — and folds
	/// them with `__arrconcat`, wrapping the result in a `$list`.
	fn make_list(&mut self, items: &[ir::ListItem]) {
		use ir::ListItem;
		if !items.iter().any(|it| matches!(it, ListItem::Spread(_))) {
			self.ins(Instruction::I32Const(types::TAG_LIST));
			self.elems_array(
				&items
					.iter()
					.map(|it| match it {
						ListItem::Elem(a) => a.clone(),
						ListItem::Spread(_) => unreachable!(),
					})
					.collect::<Vec<_>>(),
			);
			self.ins(Instruction::StructNew(types::T_LIST));
			return;
		}
		let concat = self.runtime.idx(Helper::ArrConcat).expect("arrconcat");
		// Group items into segments: runs of plain elements vs. single spreads.
		let mut segs: Vec<Vec<&Atom>> = Vec::new();
		let mut spread_at: Vec<bool> = Vec::new();
		for it in items {
			match it {
				ListItem::Elem(a) => {
					if spread_at.last() == Some(&false) {
						segs.last_mut().unwrap().push(a);
					} else {
						segs.push(vec![a]);
						spread_at.push(false);
					}
				}
				ListItem::Spread(a) => {
					segs.push(vec![a]);
					spread_at.push(true);
				}
			}
		}
		// Emit each segment's $valarray, folding with __arrconcat.
		for (i, (seg, &is_spread)) in segs.iter().zip(&spread_at).enumerate() {
			if is_spread {
				self.atom(seg[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
			} else {
				for a in seg {
					self.atom(a);
				}
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: seg.len() as u32,
				});
			}
			if i > 0 {
				self.ins(Instruction::Call(concat));
			}
		}
		let tmp = self.fresh_local(types::valarray_ref());
		self.ins(Instruction::LocalSet(tmp));
		self.ins(Instruction::I32Const(types::TAG_LIST));
		self.ins(Instruction::LocalGet(tmp));
		self.ins(Instruction::StructNew(types::T_LIST));
	}

	fn atom_repr(&self, a: &Atom) -> Repr {
		match a {
			Atom::Var(v) => self
				.f
				.var_reprs
				.get(v.0 as usize)
				.copied()
				.unwrap_or(Repr::Boxed),
			Atom::Const(Const::Int(_)) => Repr::I64,
			Atom::Const(Const::Float(_)) => Repr::F64,
			Atom::Const(Const::Bool(_)) => Repr::I32,
			Atom::Const(_) => Repr::Boxed,
		}
	}

	fn local(&self, var: u32) -> u32 {
		self.locals[var as usize]
	}

	fn ins(&mut self, ins: Instruction<'static>) {
		self.body.push(ins);
	}
}
