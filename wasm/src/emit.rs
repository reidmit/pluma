// Per-function body emission. `FnEmitter` walks one function's IR `Block`,
// mapping each `VarId` to a wasm local and each `Rvalue` to GC/numeric
// instructions, and produces a `wasm_encoder::Function`. The arity-keyed
// uniform-boxed contract (see `lib.rs`) means a function's wasm signature is
// fixed by its arity; `var_reprs` says which locals are unboxed i64/f64/i32 and
// `Box`/`Unbox` mark the GC-ref boundaries the coercion pass already inserted.

use crate::Diagnostics;
use crate::async_lower::TASK_ENUM;
use crate::runtime::{
	ClockKind, DomKind, GlobalKind, GlobalSlot, Helper, IoKind, RngKind, Runtime, WIRE_FNV_OFFSET,
	clock_kind, dom_kind, host_sig, io_kind, is_byte_writer, is_clock_host, is_dom_host,
	is_f64_unary_host, is_inline_builtin, is_io_host, is_net_sync, is_raw_writer, is_rng_host,
	rng_kind, task_builtin_kind, task_kind,
};
use crate::scan::{StrPool, block_has_pushdefer, builtin_var_tags, compute_nominal, ctor_var_tags};
use crate::types::{self, FuncTypes};
use crate::util::{EnumTable, binop_instr, repr_valtype, variant_display};
use ir::{Atom, Block, Callee, Const, Repr, Rvalue, StmtKind};
use std::collections::{HashMap, HashSet};
use wasm_encoder::*;

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
	/// Each function's return `Repr`, indexed by `FuncId.0`. A direct tail call
	/// (`return_call`) is valid only when this function's return repr equals the
	/// callee's; on a mismatch (numeric monomorphization made them differ) the tail
	/// call is downgraded to a plain call so the coercer's bridging `Return` runs.
	callee_rets: &'a [Repr],
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
	/// Source line (0-based) of the statement currently being emitted, refreshed
	/// per `Stmt` in `block`. Only consumed by `debug`, which renders a
	/// `[<module>:<line>]` call-site header.
	cur_line: usize,
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
		extra_nominal: &'a HashMap<u32, Vec<(u32, ir::RecordShape)>>,
		callee_rets: &'a [Repr],
		extra_params: u32,
		diags: &'a mut Diagnostics,
	) -> Self {
		let var_tags = builtin_var_tags(&f.body, builtin_g);
		let var_ctors = ctor_var_tags(&f.body);
		let nominal = compute_nominal(f, fid, param_shapes, extra_nominal);
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
			callee_rets,
			strpool,
			diags,
			locals,
			local_types,
			next_local: next,
			depth: 0,
			loop_stack: Vec::new(),
			defers_local: None,
			cur_line: 0,
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
			self.ins(Instruction::ArrayNewFixed {
				array_type_index: types::T_VALARRAY,
				array_size: 0,
			});
			self.mk_list();
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
			self.cur_line = s.range.start.line;
			self.stmt(&s.kind);
		}
	}

	fn stmt(&mut self, k: &StmtKind) {
		match k {
			StmtKind::Let(v, rv) => {
				// A record producer bound to a nominal var builds a `$shapeN` struct
				// (constant-index reads); otherwise the rvalue emits its uniform form.
				match rv {
					Rvalue::MakeRecord(fields, _) if self.nominal.contains_key(&v.0) => {
						let shape = self.nominal[&v.0].clone();
						self.make_record_nominal(&shape, fields);
					}
					Rvalue::RecordUpdate { base, fields, .. } if self.nominal.contains_key(&v.0) => {
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
				// Run scheduled `defer` cleanups (LIFO) before returning — the
				// frame's cleanup stack runs at `Return`. The return
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
				// singleton `$valarray` [thunk].
				self.atom(a);
				self.ins(Instruction::ArrayNewFixed {
					array_type_index: types::T_VALARRAY,
					array_size: 1,
				});
				// defers.elems (a defers list is never `push`ed, so length == capacity).
				self.ins(Instruction::LocalGet(dl));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
				self.ins(Instruction::Call(concat));
				self.mk_list();
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
			Variant { tag, fields, .. } => self.test_variant(*tag, fields, subj, fail_level),
			Tuple(elems) => {
				// A tuple's arity is fixed by its type — no tag/length check. Elements
				// are read from the inline slots.
				for (i, sub) in elems.iter().enumerate() {
					self.bind_tuple_elem(sub, subj, i, fail_level);
				}
			}
			List { items, rest } => {
				// Length: exact (== items) when no rest, else at-least (>= items).
				self.ins(Instruction::LocalGet(subj));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				// the logical length (field 2), not array.len (capacity).
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 2,
				});
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
					let st = self.ftypes.intern_shape(&sshape).type_idx;
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
							self.nominal_field_boxed(subj, st, slot, sshape.field_reprs[slot]);
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
						self.nominal_field_boxed(subj, st, slot, sshape.field_reprs[slot]);
						self.ins(Instruction::LocalSet(tmp));
						self.test_pattern(sub, tmp, None, fail_level);
					}
					return;
				}
				// Uniform subject: name-scan via `__getfield`. A nominal `$shapeN` reaching
				// here (a record that flowed through generic code) is lifted first — both
				// for this exact-arity check and so the `__getfield` reads below hit a
				// `$record` (they self-lift too, but the arity check casts directly).
				if let ir::RecordRest::Exact = rest {
					let denom = self
						.runtime
						.idx(Helper::Denominalize)
						.expect("denominalize");
					self.ins(Instruction::LocalGet(subj));
					self.ins(Instruction::Call(denom));
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
					for (name, _) in fields {
						self.string_const(name);
					}
					self.ins(Instruction::ArrayNewFixed {
						array_type_index: types::T_VALARRAY,
						array_size: fields.len() as u32,
					});
					self.mk_list();
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

	/// Read field `slot` off nominal record `subj:st` and leave a *boxed* `$value`:
	/// an unboxed `F64` slot is boxed into a `$float` so it can flow into a uniform
	/// `$valarray` or a boxed pattern binding; a `Boxed` slot is already a value.
	fn nominal_field_boxed(&mut self, subj: u32, st: u32, slot: usize, repr: ir::Repr) {
		if repr == ir::Repr::F64 {
			// Box order mirrors `Rvalue::Box(F64)`: tag, then the `f64`, then `struct.new`.
			self.ins(Instruction::I32Const(types::TAG_FLOAT));
			self.nominal_field(subj, st, slot);
			self.ins(Instruction::StructNew(types::T_FLOAT));
		} else {
			self.nominal_field(subj, st, slot);
		}
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
		let Some(denom) = self.runtime.idx(Helper::Denominalize) else {
			return;
		};
		// `rec` may be a nominal `$shapeN` (this guard also shadows the uniform
		// name-scan path, which now sees nominal records); lift it before casting.
		self.ins(Instruction::LocalGet(rec));
		self.ins(Instruction::Call(denom));
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
				// The subject may be an `i31ref` (small int) or a heap `$int`.
				self.ins(Instruction::LocalGet(subj));
				self.unbox_int();
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

	fn test_variant(&mut self, tag: u32, fields: &[ir::Pattern], subj: u32, fail_level: u32) {
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
		// Bind / recurse on payload fields, read from the inline slots. The arity is
		// the pattern's field count, so the read is a constant-field `struct.get`.
		let arity = fields.len();
		for (i, field) in fields.iter().enumerate() {
			self.bind_variant_field(field, subj, arity, i, fail_level);
		}
	}

	/// Push payload element `i` of the variant in local `subj` onto the stack. With
	/// the payload inline, arity ≤ 2 reads `p0`/`p1` (fields 4/5) directly; arity ≥ 3
	/// reads the `rest` overflow array (field 6). `arity` is statically known at
	/// every call site (a pattern's field count, or a constructor's declared arity).
	fn get_variant_elem(&mut self, subj: u32, arity: usize, i: usize) {
		self.ins(Instruction::LocalGet(subj));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_VARIANT,
		)));
		if arity <= 2 {
			self.ins(Instruction::StructGet {
				struct_type_index: types::T_VARIANT,
				field_index: 4 + i as u32,
			});
		} else {
			self.ins(Instruction::StructGet {
				struct_type_index: types::T_VARIANT,
				field_index: 6,
			});
			self.ins(Instruction::I32Const(i as i32));
			self.ins(Instruction::ArrayGet(types::T_VALARRAY));
		}
	}

	/// Bind (or recursively match) variant payload field `i`, read inline. Mirrors
	/// `bind_at`, but sources the element from `get_variant_elem`.
	fn bind_variant_field(
		&mut self,
		sub: &ir::Pattern,
		subj: u32,
		arity: usize,
		i: usize,
		fail: u32,
	) {
		match sub {
			ir::Pattern::Wildcard => {}
			ir::Pattern::Bind(v) => {
				let dst = self.local(v.0);
				self.get_variant_elem(subj, arity, i);
				self.ins(Instruction::LocalSet(dst));
			}
			other => {
				let tmp = self.fresh_local(types::value_ref());
				self.get_variant_elem(subj, arity, i);
				self.ins(Instruction::LocalSet(tmp));
				self.test_pattern(other, tmp, None, fail);
			}
		}
	}

	/// Construct a `$tuple` with its elements inline. Arity ≤ 3 stores them in
	/// `e0`/`e1`/`e2` (`rest` null) — one struct, no elems array; arity ≥ 4 keeps the
	/// first three inline and the overflow in `rest`.
	fn emit_make_tuple(&mut self, elems: &[Atom]) {
		let n = elems.len();
		self.ins(Instruction::I32Const(types::TAG_TUPLE));
		self.ins(Instruction::I32Const(n as i32));
		for slot in 0..3 {
			if let Some(a) = elems.get(slot) {
				self.atom(a);
			} else {
				self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
			}
		}
		if n <= 3 {
			self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALARRAY)));
		} else {
			for a in &elems[3..] {
				self.atom(a);
			}
			self.ins(Instruction::ArrayNewFixed {
				array_type_index: types::T_VALARRAY,
				array_size: (n - 3) as u32,
			});
		}
		self.ins(Instruction::StructNew(types::T_TUPLE));
	}

	/// Push tuple element `i` of the tuple in local `subj`. Elements are inline at a
	/// fixed position: field `2 + i` for `i < 3`, else the `rest` overflow (field 5).
	/// The position depends only on `i`, so the arity need not be known here.
	fn get_tuple_elem(&mut self, subj: u32, i: usize) {
		self.ins(Instruction::LocalGet(subj));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_TUPLE,
		)));
		if i < 3 {
			self.ins(Instruction::StructGet {
				struct_type_index: types::T_TUPLE,
				field_index: 2 + i as u32,
			});
		} else {
			self.ins(Instruction::StructGet {
				struct_type_index: types::T_TUPLE,
				field_index: 5,
			});
			self.ins(Instruction::I32Const((i - 3) as i32));
			self.ins(Instruction::ArrayGet(types::T_VALARRAY));
		}
	}

	/// Bind (or recursively match) tuple element `i`, read inline. Mirrors `bind_at`.
	fn bind_tuple_elem(&mut self, sub: &ir::Pattern, subj: u32, i: usize, fail: u32) {
		match sub {
			ir::Pattern::Wildcard => {}
			ir::Pattern::Bind(v) => {
				let dst = self.local(v.0);
				self.get_tuple_elem(subj, i);
				self.ins(Instruction::LocalSet(dst));
			}
			other => {
				let tmp = self.fresh_local(types::value_ref());
				self.get_tuple_elem(subj, i);
				self.ins(Instruction::LocalSet(tmp));
				self.test_pattern(other, tmp, None, fail);
			}
		}
	}

	/// Construct a `$variant` with its payload inline (the hot path: every user-code
	/// variant build). Arity ≤ 2 stores the elements in `p0`/`p1` (`rest` null) — one
	/// struct, no payload array; arity ≥ 3 (rare) stores the whole payload in `rest`.
	fn emit_make_variant(&mut self, enum_name: &str, tag: u32, payload: &[Atom]) {
		let n = payload.len();
		self.ins(Instruction::I32Const(types::TAG_VARIANT));
		self.ins(Instruction::I32Const(tag as i32));
		self.string_const(&variant_display(enum_name, tag, self.enums));
		self.ins(Instruction::I32Const(n as i32));
		if n <= 2 {
			for slot in 0..2 {
				if let Some(a) = payload.get(slot) {
					self.atom(a);
				} else {
					self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
				}
			}
			self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALARRAY)));
		} else {
			self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
			self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
			for a in payload {
				self.atom(a);
			}
			self.ins(Instruction::ArrayNewFixed {
				array_type_index: types::T_VALARRAY,
				array_size: n as u32,
			});
		}
		self.ins(Instruction::StructNew(types::T_VARIANT));
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
				// f64 has no remainder opcode; compute `a - trunc(a/b)*b` —
				// Rust/IEEE `fmod` semantics for normal-magnitude operands.
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
				// `int` boxes through `box_int` (i31 immediate when small); `float`/
				// `bool` are always heap `$float`/`$bool` structs.
				let repr = self.atom_repr(a);
				match repr {
					Repr::I64 => {
						self.atom(a);
						self.box_int();
					}
					Repr::F64 => {
						self.ins(Instruction::I32Const(types::TAG_FLOAT));
						self.atom(a);
						self.ins(Instruction::StructNew(types::T_FLOAT));
					}
					Repr::I32 => {
						self.ins(Instruction::I32Const(types::TAG_BOOL));
						self.atom(a);
						self.ins(Instruction::StructNew(types::T_BOOL));
					}
					Repr::Boxed => {
						self.diags.push("Box of an already-boxed value");
					}
				}
			}
			Rvalue::Unbox(a, repr) => {
				self.atom(a);
				self.unbox_value(*repr);
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
			Rvalue::Call(Callee::Builtin(tag, ret), args) => {
				// A builtin call resolved to its static target by
				// `ir::resolve::resolve_builtins`, carrying its declared return repr.
				// When that repr is unboxed, produce the scalar directly: the hot inline
				// scalars (`bytes-get` per byte, `bytes-length`, `list-length`) skip the
				// `$int` box entirely — the whole point of this pass. Any other unboxed-
				// returning builtin falls back to the boxed dispatch plus one unbox (its
				// box is host-built and unavoidable, but the result still lands in the
				// unboxed local the repr pass expects). The coercer's `Box` nodes rebox
				// only where the result is actually consumed boxed.
				if *ret != Repr::Boxed && self.try_emit_unboxed_builtin(tag, args) {
					// emitted the bare scalar — nothing more to do.
				} else {
					self.host_call(tag, args);
					if *ret != Repr::Boxed {
						self.unbox_value(*ret);
					}
				}
			}
			Rvalue::TailCallDirect(fid, args) => {
				let Some(&w) = self.wasm_index.get(&fid.0) else {
					self
						.diags
						.push(format!("tail call to unreachable fn {}", fid.0));
					self.push_nothing();
					return;
				};
				// Same direct-call convention as `Call(Callee::Function)`: null env,
				// then shape-aware args. A tail call would `return_call` past the
				// trailing `Return`, skipping `defer` cleanups, so downgrade to an
				// ordinary call in a defer-bearing function (mirrors `TailCall`).
				// `return_call` also requires the callee's return type to match this
				// function's; numeric monomorphization can make them differ (an unboxed
				// caller tail-calling a boxed callee, or vice-versa), so downgrade then
				// too and let the coercer's trailing `Return` bridge the reprs.
				let ret_matches = self
					.callee_rets
					.get(fid.0 as usize)
					.is_none_or(|r| *r == self.f.ret_repr);
				self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
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
				if self.defers_local.is_none() && ret_matches {
					self.ins(Instruction::ReturnCall(w));
				} else {
					self.ins(Instruction::Call(w));
				}
			}
			Rvalue::CallClosure(callee, args) => self.call_value(callee, args, false),
			Rvalue::TailCall(callee, args) => {
				// A tail call would `return_call` past the trailing `Return`, skipping
				// any `defer` cleanups — so in a defer-bearing function, downgrade it
				// to an ordinary call and let the `Return` run the cleanups (TCO is
				// suppressed while a frame has pending cleanups). An indirect call yields
				// a boxed value, so `return_call_indirect` is only type-valid when this
				// function also returns boxed; a monomorphized unboxed-return caller
				// downgrades and lets the coercer's `Return` box the result.
				let tail = self.defers_local.is_none() && self.f.ret_repr == Repr::Boxed;
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
			} if enum_name == TASK_ENUM => {
				// The async-fn lowering builds a `$task` (not a `$variant`); `tag` is
				// the `task_kind` discriminant.
				self.make_task(*tag as i32, payload);
			}
			Rvalue::MakeVariant {
				enum_name,
				tag,
				payload,
			} => {
				self.emit_make_variant(enum_name, *tag, payload);
			}
			Rvalue::MakeVariantCtor { tag, enum_name } => {
				let arity = self.variant_arity(enum_name, *tag);
				self.ins(Instruction::I32Const(types::TAG_CTOR));
				self.ins(Instruction::I32Const(*tag as i32));
				self.ins(Instruction::I32Const(arity as i32));
				self.ins(Instruction::StructNew(types::T_CTOR));
			}
			Rvalue::MakeTuple(elems) => self.emit_make_tuple(elems),
			Rvalue::MakeList(items) => self.make_list(items),
			Rvalue::MakeRecord(fields, _) => {
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
							let st = self.ftypes.intern_shape(&rshape).type_idx;
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
				// Uniform path: name-scan via `__getfield`, which self-lifts a nominal
				// `$shapeN` receiver (one that flowed through generic code) to the uniform
				// `$record` before scanning. A statically-nominal receiver already took the
				// constant-index fast path above.
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
			Rvalue::RecordUpdate { base, fields, .. } => {
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
				// Payload is inline; without a statically-known arity here, route
				// through `__variant_payload` (materializes the uniform array) then
				// index. Cold: the IR currently never produces `GetPayload` (pattern
				// matching reads inline via `get_variant_elem`).
				let Some(vp) = self.runtime.idx(Helper::VariantPayload) else {
					self
						.diags
						.push("GetPayload used but __variant_payload not emitted");
					return;
				};
				self.atom(a);
				self.ins(Instruction::Call(vp));
				self.ins(Instruction::I32Const(*i as i32));
				self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			}
			Rvalue::GetElement(a, i) => {
				// Elements are inline at a fixed position: field `2 + i` for `i < 3`,
				// else the `rest` overflow. No arity needed (position is set by `i`).
				self.atom(a);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_TUPLE,
				)));
				if *i < 3 {
					self.ins(Instruction::StructGet {
						struct_type_index: types::T_TUPLE,
						field_index: 2 + *i,
					});
				} else {
					self.ins(Instruction::StructGet {
						struct_type_index: types::T_TUPLE,
						field_index: 5,
					});
					self.ins(Instruction::I32Const((*i - 3) as i32));
					self.ins(Instruction::ArrayGet(types::T_VALARRAY));
				}
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
				// Applying a constructor builds the variant directly (payload inline).
				self.emit_make_variant(&enum_name, tag, args);
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

	/// Build a `$task` `{tag: TAG_TASK, kind, payload: [args]}` — the cold async
	/// recipe the driver interprets. Used by the async-fn lowering's constructor
	/// and the `task.*` primitive builtins.
	fn make_task(&mut self, kind: i32, payload: &[Atom]) {
		self.ins(Instruction::I32Const(types::TAG_TASK));
		self.ins(Instruction::I32Const(kind));
		for a in payload {
			self.atom(a);
		}
		self.ins(Instruction::ArrayNewFixed {
			array_type_index: types::T_VALARRAY,
			array_size: payload.len() as u32,
		});
		self.ins(Instruction::StructNew(types::T_TASK));
	}

	/// Emit a scalar-returning *inline* builtin so it leaves its bare unboxed value
	/// on the wasm stack — no `$int` box. Returns `false` for any builtin without
	/// such a path, so the caller falls back to the boxed dispatch + an `unbox_value`.
	/// This is the allocation the repr threading buys back: in a per-byte loop
	/// `bytes-get` now produces an `i64` straight from `array.get_u`, never a heap box.
	/// (The boxed forms still live in `inline_builtin` for the rare unresolved/aliased
	/// builtin call, whose result repr stays `Boxed`.)
	fn try_emit_unboxed_builtin(&mut self, tag: &str, args: &[Atom]) -> bool {
		match tag {
			// bytes.get b i : (str.bytes)[i] as unsigned i64. The index arg is boxed
			// (builtin args are uniform), so unbox it inline as before.
			"bytes-get" => {
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_STR,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_STR,
					field_index: 1,
				});
				self.atom(&args[1]);
				self.unbox_int();
				self.ins(Instruction::I32WrapI64);
				self.ins(Instruction::ArrayGetU(types::T_BYTES));
				self.ins(Instruction::I64ExtendI32U);
				true
			}
			// bytes.length b : array.len of the str's bytes, as i64.
			"bytes-length" => {
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
				true
			}
			// list.length xs : the list's logical length (field 2), as i64.
			"list-length" => {
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 2,
				});
				self.ins(Instruction::I64ExtendI32U);
				true
			}
			_ => false,
		}
	}

	/// Unbox the boxed `$value` on top of the stack into the scalar `repr` demands
	/// (`$int`/`$float`/`$bool` payload). Mirrors the `Rvalue::Unbox` lowering, for
	/// coercing a boxed builtin result into the unboxed local the repr pass assigned.
	fn unbox_value(&mut self, repr: Repr) {
		// `int` rides as either an `i31ref` immediate (small) or a heap `$int`;
		// `unbox_int` discriminates. `float`/`bool` are always heap structs.
		let (ty, field) = match repr {
			Repr::I64 => {
				self.unbox_int();
				return;
			}
			Repr::F64 => (types::T_FLOAT, 1),
			Repr::I32 => (types::T_BOOL, 1),
			Repr::Boxed => {
				self.diags.push("unbox_value to Boxed");
				return;
			}
		};
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(ty)));
		self.ins(Instruction::StructGet {
			struct_type_index: ty,
			field_index: field,
		});
	}

	/// Box an i64 (on the stack) into a value: a signed-31-bit-representable int
	/// becomes an `i31ref` immediate (no heap allocation, not refcounted by the DRC
	/// collector), else a heap `$int`. The `eqref`-rooted value type makes the i31
	/// a valid value everywhere a box flows.
	fn box_int(&mut self) {
		let t = self.fresh_local(ValType::I64);
		self.ins(Instruction::LocalSet(t));
		self.ins(Instruction::LocalGet(t));
		self.ins(Instruction::I64Const(-(1 << 30)));
		self.ins(Instruction::I64GeS);
		self.ins(Instruction::LocalGet(t));
		self.ins(Instruction::I64Const(1 << 30));
		self.ins(Instruction::I64LtS);
		self.ins(Instruction::I32And);
		self.ins(Instruction::If(BlockType::Result(types::value_ref())));
		self.ins(Instruction::LocalGet(t));
		self.ins(Instruction::I32WrapI64);
		self.ins(Instruction::RefI31);
		self.ins(Instruction::Else);
		self.ins(Instruction::I32Const(types::TAG_INT));
		self.ins(Instruction::LocalGet(t));
		self.ins(Instruction::StructNew(types::T_INT));
		self.ins(Instruction::End);
	}

	/// Unbox a value known to be an `int` (on the stack) to a bare i64, handling
	/// both the `i31ref` immediate and the heap `$int` forms (so a small int and a
	/// large/host-built one compare and arithmetic alike).
	fn unbox_int(&mut self) {
		let t = self.fresh_local(types::value_ref());
		self.ins(Instruction::LocalSet(t));
		self.ins(Instruction::LocalGet(t));
		self.ins(Instruction::RefTestNonNull(HeapType::I31));
		self.ins(Instruction::If(BlockType::Result(ValType::I64)));
		self.ins(Instruction::LocalGet(t));
		self.ins(Instruction::RefCastNonNull(HeapType::I31));
		self.ins(Instruction::I31GetS);
		self.ins(Instruction::I64ExtendI32S);
		self.ins(Instruction::Else);
		self.ins(Instruction::LocalGet(t));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_INT,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_INT,
			field_index: 1,
		});
		self.ins(Instruction::End);
	}

	fn host_call(&mut self, tag: &str, args: &[Atom]) {
		// `debug x` prints a `[<module>:<line>]` header then returns `x` unchanged.
		// Emitted inline (the call site is known statically) rather than imported.
		if tag == "debug" {
			self.emit_debug(args);
			return;
		}
		// Pure-compute builtins emitted inline over the `$value` GC layout.
		if is_inline_builtin(tag) {
			self.inline_builtin(tag, args);
			return;
		}
		// `task.*` / `scope-new`/`scope-next` pure constructors build a cold `$task`
		// directly (the scheduler in `helpers/task.rs` runs it). The suspending net
		// ops (`net-accept`/`net-read`/`net-write`) are `$task` kinds too — the
		// scheduler does their host call + reactor park.
		if let Some(kind) = task_builtin_kind(tag) {
			self.make_task(kind, args);
			return;
		}
		// Synchronous `std/sys/net` ops (`listen`/`close`/`local-addr`/`connect`): a host
		// call shaped into a `result` here (connect additionally wraps it in a Pure
		// `$task`, matching its `task (result …)` type).
		if is_net_sync(tag) {
			self.emit_net_sync(tag, args);
			return;
		}
		// The side-effecting scope-kernel ops call straight into the scheduler:
		// `s.spawn` registers a child fiber (returns a HANDLE task); `s.cancel` /
		// `s.cancel-after` queue a cancellation / arm a deadline timer (return ()).
		if let Some(h) = match tag {
			"scope-spawn" => Some(self.runtime.idx(Helper::SchedSpawn)),
			"scope-cancel" => Some(self.runtime.idx(Helper::SchedCancel)),
			"scope-cancel-after" => Some(self.runtime.idx(Helper::SchedCancelAfter)),
			_ => None,
		} {
			match h {
				Some(h) => {
					for a in args {
						self.atom(a);
					}
					self.ins(Instruction::Call(h));
				}
				None => {
					self
						.diags
						.push(format!("`{tag}` needs its scheduler helper"));
					self.push_nothing();
				}
			}
			return;
		}
		// Task-local cells (`std/local`). All three lower to scheduler helpers — the
		// driver runs for every program, so the helpers always exist. `local.get`'s
		// helper falls back to the cell's `default` when no binding is in scope (incl.
		// a fully sync `main`, where the scheduler never spun up).
		match tag {
			"local-enter" | "local-exit" => {
				let h = if tag == "local-enter" {
					self.runtime.idx(Helper::LocalEnter)
				} else {
					self.runtime.idx(Helper::LocalExit)
				};
				match h {
					Some(h) => {
						for a in args {
							self.atom(a);
						}
						self.ins(Instruction::Call(h));
					}
					None => {
						self
							.diags
							.push(format!("`{tag}` needs its scheduler helper"));
						self.push_nothing();
					}
				}
				return;
			}
			"local-get" => {
				match self.runtime.idx(Helper::LocalGet) {
					Some(h) => {
						self.atom(&args[0]);
						self.ins(Instruction::Call(h));
					}
					None => {
						self
							.diags
							.push(format!("`{tag}` needs its scheduler helper"));
						self.push_nothing();
					}
				}
				return;
			}
			_ => {}
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
		// Higher-order builders + `list.push`: synthetic helpers called with the
		// boxed args (a loop + closure call for the builders; an in-place append
		// for push), leaving the result (or `nothing` for push) on the stack.
		if tag == "list-build"
			|| tag == "list-collect"
			|| tag == "bytes-build"
			|| tag == "list-push"
			|| tag == "spawn-command"
			|| tag == "spawn-sub"
			|| tag == "cancel-sub"
			|| tag == "rpc-stream-open"
			|| tag == "rpc-stream-close"
		{
			let helper = match tag {
				"list-build" => self.runtime.idx(Helper::ListBuild),
				"list-collect" => self.runtime.idx(Helper::ListCollect),
				"list-push" => self.runtime.idx(Helper::ListPush),
				"spawn-command" => self.runtime.idx(Helper::SpawnCommand),
				"spawn-sub" => self.runtime.idx(Helper::SpawnSub),
				"cancel-sub" => self.runtime.idx(Helper::CancelSub),
				// `std/web/stream`: open mints a channel + starts the host `fetch`; close
				// aborts it. Both build a Pure `$task` (the scheduler runs it later).
				"rpc-stream-open" => self.runtime.idx(Helper::RpcStreamOpen),
				"rpc-stream-close" => self.runtime.idx(Helper::RpcStreamClose),
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
		// dict trie ops → synthetic helpers. insert/lookup/remove receive a hash
		// method-dict as `args[0]` (the `where (hash k)` evidence); the wasm dict
		// keys on a structural `__hash` (consistent with `__eq`) computed inside the
		// helper, so that evidence arg is DROPPED — we pass only the dict + key (+
		// value). map/filter/size/entries take no hash evidence (`[dict, ...]`).
		if let Some((helper, call_args)) = match tag {
			"dict-empty" => Some((self.runtime.idx(Helper::DictEmpty), &args[0..])),
			"dict-insert" => Some((self.runtime.idx(Helper::DictInsert), &args[1..])),
			// Internal, emitted only by `ir::reuse`. `dict-mint-token` takes no args;
			// `dict-insert-into` carries the `where (hash k)` witness at args[0] (dropped,
			// like `dict-insert`) and the transient token as its last arg.
			"dict-mint-token" => Some((self.runtime.idx(Helper::DictMintToken), &args[0..])),
			"dict-insert-into" => Some((self.runtime.idx(Helper::DictInsertInto), &args[1..])),
			"dict-lookup" => Some((self.runtime.idx(Helper::DictLookup), &args[1..])),
			"dict-remove" => Some((self.runtime.idx(Helper::DictRemove), &args[1..])),
			"dict-map" => Some((self.runtime.idx(Helper::DictMap), &args[0..])),
			"dict-filter" => Some((self.runtime.idx(Helper::DictFilter), &args[0..])),
			"dict-size" => Some((self.runtime.idx(Helper::DictSize), &args[0..])),
			"dict-entries" => Some((self.runtime.idx(Helper::DictEntries), &args[0..])),
			// `dict.update` has a `where (hash k)` witness at args[0] — drop it.
			"dict-update" => Some((self.runtime.idx(Helper::DictUpdate), &args[1..])),
			"dict-clear" => Some((self.runtime.idx(Helper::DictClear), &args[0..])),
			// `dict.from-entries` has a `where (hash k)` witness at args[0] — drop it.
			"dict-from-entries" => Some((self.runtime.idx(Helper::DictFromEntries), &args[1..])),
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
			// Reuse the persistent scratch buffers across encode calls: they grow
			// once (amortized) and the write cursors reset to 0 each call. Only
			// allocate on the first encode, when the globals are still null. (The
			// result is always copied out into a fresh exact-size `$bytes`, so the
			// reused scratch never aliases a returned value.) Reallocating a fresh
			// buffer per call instead made the buffer re-grow every time, and
			// `array.copy` was slow under wasmtime — that growth churn was ~half the
			// `wire-encode` cost.
			self.ins(Instruction::GlobalGet(g.buf));
			self.ins(Instruction::RefIsNull);
			self.ins(Instruction::If(BlockType::Empty));
			self.ins(Instruction::I32Const(256));
			self.ins(Instruction::ArrayNewDefault(types::T_BYTES));
			self.ins(Instruction::GlobalSet(g.buf));
			self.ins(Instruction::End);
			self.ins(Instruction::GlobalGet(g.ctx));
			self.ins(Instruction::RefIsNull);
			self.ins(Instruction::If(BlockType::Empty));
			self.ins(Instruction::I32Const(8));
			self.ins(Instruction::ArrayNewDefault(types::T_VALARRAY));
			self.ins(Instruction::GlobalSet(g.ctx));
			self.ins(Instruction::End);
			self.ins(Instruction::I32Const(0));
			self.ins(Instruction::GlobalSet(g.len));
			self.ins(Instruction::I32Const(0));
			self.ins(Instruction::GlobalSet(g.ctxlen));
			self.atom(&args[0]);
			self.atom(&args[1]);
			self.ins(Instruction::Call(enc));
			let res = self.fresh_local(types::bytes_ref());
			self.ins(Instruction::GlobalGet(g.len));
			self.ins(Instruction::ArrayNewDefault(types::T_BYTES));
			self.ins(Instruction::LocalSet(res));
			// res[i] = g_buf[i] for i in 0..g_len. A manual loop, not `array.copy`
			// (slow under wasmtime over byte arrays); this final snapshot of the
			// scratch buffer was the dominant cost of `wire-encode`.
			let idx = self.fresh_local(ValType::I32);
			self.ins(Instruction::I32Const(0));
			self.ins(Instruction::LocalSet(idx));
			self.ins(Instruction::Block(BlockType::Empty));
			self.ins(Instruction::Loop(BlockType::Empty));
			self.ins(Instruction::LocalGet(idx));
			self.ins(Instruction::GlobalGet(g.len));
			self.ins(Instruction::I32GeU);
			self.ins(Instruction::BrIf(1));
			self.ins(Instruction::LocalGet(res));
			self.ins(Instruction::LocalGet(idx));
			self.ins(Instruction::GlobalGet(g.buf));
			self.ins(Instruction::LocalGet(idx));
			self.ins(Instruction::ArrayGetU(types::T_BYTES));
			self.ins(Instruction::ArraySet(types::T_BYTES));
			self.ins(Instruction::LocalGet(idx));
			self.ins(Instruction::I32Const(1));
			self.ins(Instruction::I32Add);
			self.ins(Instruction::LocalSet(idx));
			self.ins(Instruction::Br(0));
			self.ins(Instruction::End);
			self.ins(Instruction::End);
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
		// `std/web/fetch` in a browser build: the `WebFetch` helper (mint channel +
		// start the async `fetch` + return a `WEB_FETCH` task the pump pulls). Present
		// only in a browser build; the sys host falls through to the blocking path
		// below, which needs the `web-fetch` import index.
		if tag == "web-fetch" {
			if let Some(h) = self.runtime.idx(Helper::WebFetch) {
				for a in args {
					self.atom(a);
				}
				self.ins(Instruction::Call(h));
				return;
			}
		}
		let Some(&idx) = self.host_index.get(tag) else {
			self.diags.push(format!("unsupported host builtin `{tag}`"));
			self.push_nothing();
			return;
		};
		// `std/web/fetch` under the sys host: a string->string blocking host call,
		// marshalled and shaped like an io read but wrapped in a Pure `$task`.
		if tag == "web-fetch" {
			self.emit_web_fetch(idx, args);
			return;
		}
		// Byte-payload writers (`print`/`io.write*`/`io.fail`): render the arg into the
		// scratch memory and pass `(ptr=0, len)` to the `(i32,i32) -> ()` host import.
		if is_byte_writer(tag) {
			self.emit_byte_writer(tag, idx, &args[0]);
			return;
		}
		// `std/random`/`std/uuid` (except `uuid-parse`, classified as an io read):
		// box a scalar directly, or build a `$bytes`/`$str` from a scratch payload.
		if is_rng_host(tag) {
			self.emit_rng(tag, idx, args);
			return;
		}
		// `std/sys/io` host imports: marshal path/data args into scratch, call the
		// `(i32…) -> i32` import, then shape the `i32` result back into a `$value`
		// (a `$str`/`$bytes`/`$list` payload, a `nothing`/null status, or a `$bool`).
		if is_io_host(tag) {
			self.emit_io(tag, idx, args);
			return;
		}
		// `std/time` clock reads: now/monotonic box an i64 `instant`/`duration`, sleep
		// unboxes its `duration` arg, parse shapes a `result instant string`.
		if is_clock_host(tag) {
			self.emit_clock(tag, idx, args);
			return;
		}
		// `std/web/dom` (the Web target): node handles cross as `externref` (boxed into
		// / unboxed from a `$extern`), strings as scratch, and `on-click` stows its
		// handler closure in the dispatch registry.
		if is_dom_host(tag) {
			self.emit_dom(tag, idx, args);
			return;
		}
		// `io.exit code`: pass the unboxed code as a raw `i32` to the `(i32)->()` host
		// import (which exits the process). It diverges, but the `Let` binding still
		// wants a value, so materialize `nothing` after the (unreachable) call.
		if tag == "io-exit" {
			self.atom(&args[0]);
			self.unbox_int();
			self.ins(Instruction::I32WrapI64);
			self.ins(Instruction::Call(idx));
			self.push_nothing();
			return;
		}
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

	/// A byte-payload writer (`print`/`io.write*`/`io.fail`): render `arg` into the
	/// scratch memory and call its `(i32 ptr, i32 len) -> ()` host import. Formatted
	/// writers render via `__tostring` (the value's Display) then take the `$str`
	/// backing; the `*-bytes` raw writers take the `bytes` value's `$bytes` backing
	/// directly. `__send_bytes` copies it to scratch offset 0 and returns the length;
	/// the host appends a newline for the `print`-family variants. Returns `nothing`
	/// (`io.fail` traps host-side before returning).
	fn emit_byte_writer(&mut self, tag: &str, idx: u32, arg: &Atom) {
		let Some(send) = self.runtime.idx(Helper::MarshalSend) else {
			self.diags.push(format!("`{tag}` needs __send_bytes"));
			self.push_nothing();
			return;
		};
		// Push the `$bytes` to send: the raw backing, or the rendered Display.
		if is_raw_writer(tag) {
			self.str_bytes(arg);
		} else {
			let Some(ts) = self.runtime.idx(Helper::ToString) else {
				self.diags.push(format!("`{tag}` needs __tostring"));
				self.push_nothing();
				return;
			};
			self.atom(arg);
			self.ins(Instruction::Call(ts));
			self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
				types::T_STR,
			)));
			self.ins(Instruction::StructGet {
				struct_type_index: types::T_STR,
				field_index: 1,
			});
		}
		// len = __send_bytes($bytes); call writer(ptr=0, len).
		self.ins(Instruction::Call(send));
		let len = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalSet(len));
		self.ins(Instruction::I32Const(0));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::Call(idx));
		self.push_nothing();
	}

	/// Reset the scratch bump cursor to 0 — call once at the start of a host call's
	/// arg-encoding, before `__alloc`'ing this call's payloads.
	fn reset_bump(&mut self) {
		self.ins(Instruction::I32Const(0));
		self.ins(Instruction::GlobalSet(self.runtime.bump));
	}

	/// Encode a string/bytes-valued atom's `$bytes` backing into scratch: `__alloc`
	/// its length, `__store_bytes` it, and return `(ptr_local, len_local)`. The caller
	/// must have `reset_bump`'d first; successive calls bump forward so several
	/// payloads coexist for one host call (e.g. write-file's path + data).
	fn marshal_strlike_arg(&mut self, a: &Atom, alloc: u32, store: u32) -> (u32, u32) {
		let bytes_l = self.fresh_local(types::bytes_ref());
		self.str_bytes(a); // value -> $bytes (field 1 of the $str/$bytes struct)
		self.ins(Instruction::LocalSet(bytes_l));
		let len_l = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalGet(bytes_l));
		self.ins(Instruction::ArrayLen);
		self.ins(Instruction::LocalSet(len_l));
		let ptr_l = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalGet(len_l));
		self.ins(Instruction::Call(alloc));
		self.ins(Instruction::LocalSet(ptr_l));
		self.ins(Instruction::LocalGet(bytes_l));
		self.ins(Instruction::LocalGet(ptr_l));
		self.ins(Instruction::Call(store));
		(ptr_l, len_l)
	}

	/// A marshalled `std/sys/io` op: encode its path/data args into scratch, call the
	/// `(i32…) -> i32` host import, and shape the `i32` result back into a `$value`.
	/// Reads length-probe a `dst` buffer (an overflow beyond the initial cap drains
	/// the host's stash via `__io_copyout`); writers wrap a `status` into `ok nothing`
	/// / `err`; the queries box a `bool`. `__io_result` does the `ok`/`err` wrapping
	/// (a null payload = failure), so the host never builds the `result` enum.
	/// `std/random`/`std/uuid` payload builders (all but `uuid-parse`, which rides
	/// `emit_io`). The scalars box the host's `i64`/`f64` return directly; `random-bytes`
	/// and `uuid-v4`/`v7` fill scratch and build a `$bytes`/`$str`. Validation lives in
	/// Pluma, so none of these fail (no `result` wrap).
	fn emit_rng(&mut self, tag: &str, idx: u32, args: &[Atom]) {
		match rng_kind(tag) {
			Some(RngKind::ScalarI64) => {
				self.ins(Instruction::Call(idx)); // () -> i64
				self.box_int();
			}
			Some(RngKind::ScalarF64) => {
				self.ins(Instruction::I32Const(types::TAG_FLOAT));
				self.ins(Instruction::Call(idx)); // () -> f64
				self.ins(Instruction::StructNew(types::T_FLOAT));
			}
			Some(RngKind::RangeI64) => {
				self.atom(&args[0]);
				self.unbox_int();
				self.atom(&args[1]);
				self.unbox_int();
				self.ins(Instruction::Call(idx)); // (i64, i64) -> i64
				self.box_int();
			}
			Some(RngKind::BytesN) => self.emit_rng_read(tag, idx, Some(&args[0]), types::TAG_BYTES),
			Some(RngKind::UuidStr) => self.emit_rng_read(tag, idx, None, types::TAG_STR),
			None => {
				self.diags.push(format!("`{tag}` is not an rng builtin"));
				self.push_nothing();
			}
		}
	}

	/// The scratch-payload read for `random-bytes`/`uuid-v4`/`uuid-v7`: `dst = __alloc(cap)`;
	/// call `(n?, dst, cap) -> len`; drain the overflow stash if it didn't fit (`random-bytes`
	/// only — uuid is fixed-width); build the `$bytes`/`$str` from `(dst, len)`. No
	/// `__io_result` wrap — these never fail.
	fn emit_rng_read(&mut self, tag: &str, idx: u32, n_arg: Option<&Atom>, tag_const: i32) {
		let (Some(alloc), Some(load)) = (
			self.runtime.idx(Helper::MarshalAlloc),
			self.runtime.idx(Helper::MarshalLoad),
		) else {
			self
				.diags
				.push(format!("`{tag}` needs the marshalling helpers"));
			self.push_nothing();
			return;
		};
		const CAP: i32 = 4096;
		self.reset_bump();
		// `random-bytes` passes its (Pluma-validated, non-negative) length first.
		let n_local = n_arg.map(|a| {
			self.atom(a);
			self.unbox_int();
			self.ins(Instruction::I32WrapI64);
			let l = self.fresh_local(ValType::I32);
			self.ins(Instruction::LocalSet(l));
			l
		});
		let dst = self.fresh_local(ValType::I32);
		self.ins(Instruction::I32Const(CAP));
		self.ins(Instruction::Call(alloc));
		self.ins(Instruction::LocalSet(dst));
		if let Some(n) = n_local {
			self.ins(Instruction::LocalGet(n));
		}
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::I32Const(CAP));
		self.ins(Instruction::Call(idx)); // -> i32 len
		let len = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalSet(len));
		// Overflow: re-`__alloc` the true size and drain the host stash (registered only
		// when a `random-bytes` is reachable; uuid always fits `CAP`).
		if let Some(copyout) = self.host_index.get("io-copyout").copied() {
			self.ins(Instruction::LocalGet(len));
			self.ins(Instruction::I32Const(CAP));
			self.ins(Instruction::I32GtS);
			self.ins(Instruction::If(BlockType::Empty));
			self.ins(Instruction::LocalGet(len));
			self.ins(Instruction::Call(alloc));
			self.ins(Instruction::LocalSet(dst));
			self.ins(Instruction::LocalGet(dst));
			self.ins(Instruction::Call(copyout));
			self.ins(Instruction::End);
		}
		// `$str`/`$bytes` share the `{tag, $bytes}` struct; only the tag differs.
		self.ins(Instruction::I32Const(tag_const));
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::Call(load));
		self.ins(Instruction::StructNew(types::T_STR));
	}

	/// Unbox a node-valued atom to its raw `externref`: cast to the `$extern` wrapper
	/// and read its handle field. Leaves an `externref` on the stack.
	fn unbox_extern(&mut self, a: &Atom) {
		self.atom(a);
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_EXTERN,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_EXTERN,
			field_index: 1,
		});
	}

	/// `std/web/dom` host imports (`the Web target`). Node handles cross as `externref`
	/// (unboxed from / boxed into a `$extern`); string args ride scratch `(ptr, len)`;
	/// `on-click` stows its handler closure in the dispatch registry and passes a token.
	/// A node-returning import (`Body`/`Make`) pushes `TAG_EXTERN` *under* the call so the
	/// post-call stack reads `[tag, handle]` for `struct.new $extern` (like `emit_clock_scalar`).
	fn emit_dom(&mut self, tag: &str, idx: u32, args: &[Atom]) {
		let kind = match dom_kind(tag) {
			Some(k) => k,
			None => {
				self.diags.push(format!("emit_dom on non-dom tag `{tag}`"));
				self.push_nothing();
				return;
			}
		};
		// The string-marshalling helpers (present whenever a string-carrying dom
		// builtin is reachable — `module.rs` requests them alongside the dom imports).
		let (alloc, store, load) = (
			self.runtime.idx(Helper::MarshalAlloc),
			self.runtime.idx(Helper::MarshalStore),
			self.runtime.idx(Helper::MarshalLoad),
		);
		match kind {
			// `() -> externref`: box the returned node.
			DomKind::Body => {
				self.ins(Instruction::I32Const(types::TAG_EXTERN));
				self.ins(Instruction::Call(idx));
				self.ins(Instruction::StructNew(types::T_EXTERN));
			}
			// `(ptr, len) -> externref`: one string in, box the returned node.
			DomKind::Make => {
				let (Some(alloc), Some(store)) = (alloc, store) else {
					self
						.diags
						.push(format!("`{tag}` needs the marshalling helpers"));
					self.push_nothing();
					return;
				};
				self.reset_bump();
				let (ptr, len) = self.marshal_strlike_arg(&args[0], alloc, store);
				self.ins(Instruction::I32Const(types::TAG_EXTERN));
				self.ins(Instruction::LocalGet(ptr));
				self.ins(Instruction::LocalGet(len));
				self.ins(Instruction::Call(idx));
				self.ins(Instruction::StructNew(types::T_EXTERN));
			}
			// `(externref, externref) -> ()`.
			DomKind::Append => {
				self.unbox_extern(&args[0]);
				self.unbox_extern(&args[1]);
				self.ins(Instruction::Call(idx));
				self.push_nothing();
			}
			// `(externref, ptr, len) -> ()`. Marshal the string first (it writes scratch),
			// then push the node handle so the stack is `[externref, ptr, len]`.
			DomKind::SetText => {
				let (Some(alloc), Some(store)) = (alloc, store) else {
					self
						.diags
						.push(format!("`{tag}` needs the marshalling helpers"));
					self.push_nothing();
					return;
				};
				self.reset_bump();
				let (ptr, len) = self.marshal_strlike_arg(&args[1], alloc, store);
				self.unbox_extern(&args[0]);
				self.ins(Instruction::LocalGet(ptr));
				self.ins(Instruction::LocalGet(len));
				self.ins(Instruction::Call(idx));
				self.push_nothing();
			}
			// `(externref, np, nl, vp, vl) -> ()`: node + two scratch strings. `SetProp`
			// (`node[name] = value`, a property write) is identical here — the only
			// difference is the host import the call lands on.
			DomKind::SetAttr | DomKind::SetProp => {
				let (Some(alloc), Some(store)) = (alloc, store) else {
					self
						.diags
						.push(format!("`{tag}` needs the marshalling helpers"));
					self.push_nothing();
					return;
				};
				self.reset_bump();
				let (np, nl) = self.marshal_strlike_arg(&args[1], alloc, store);
				let (vp, vl) = self.marshal_strlike_arg(&args[2], alloc, store);
				self.unbox_extern(&args[0]);
				self.ins(Instruction::LocalGet(np));
				self.ins(Instruction::LocalGet(nl));
				self.ins(Instruction::LocalGet(vp));
				self.ins(Instruction::LocalGet(vl));
				self.ins(Instruction::Call(idx));
				self.push_nothing();
			}
			// `(externref, np, nl, i32) -> ()`: node + a name string + the unboxed bool.
			// Marshal the name, push the node handle, then unbox the `bool` arg to an i32
			// (`node[name] = !!flag` on the host).
			DomKind::SetBoolProp => {
				let (Some(alloc), Some(store)) = (alloc, store) else {
					self
						.diags
						.push(format!("`{tag}` needs the marshalling helpers"));
					self.push_nothing();
					return;
				};
				self.reset_bump();
				let (np, nl) = self.marshal_strlike_arg(&args[1], alloc, store);
				self.unbox_extern(&args[0]);
				self.ins(Instruction::LocalGet(np));
				self.ins(Instruction::LocalGet(nl));
				self.atom(&args[2]);
				self.unbox_value(Repr::I32); // bool box -> i32
				self.ins(Instruction::Call(idx));
				self.push_nothing();
			}
			// `(externref, dst, cap) -> len`: probe-read the node's `.value` into a `$str`.
			DomKind::GetValue => {
				let (Some(alloc), Some(load)) = (alloc, load) else {
					self
						.diags
						.push(format!("`{tag}` needs the marshalling helpers"));
					self.push_nothing();
					return;
				};
				self.emit_dom_get_value(idx, &args[0], alloc, load);
			}
			// `(externref node, i32 np, i32 nl, i32 token) -> ()`: marshal the event-name
			// string, register the handler closure, pass `(node, name, token)`.
			DomKind::Listen => {
				let (Some(alloc), Some(store)) = (alloc, store) else {
					self
						.diags
						.push(format!("`{tag}` needs the marshalling helpers"));
					self.push_nothing();
					return;
				};
				let Some(register) = self.runtime.idx(Helper::DomRegister) else {
					self.diags.push(format!("`{tag}` needs __dom_register"));
					self.push_nothing();
					return;
				};
				self.reset_bump();
				let (np, nl) = self.marshal_strlike_arg(&args[1], alloc, store);
				self.unbox_extern(&args[0]);
				self.ins(Instruction::LocalGet(np));
				self.ins(Instruction::LocalGet(nl));
				self.atom(&args[2]); // the handler closure
				self.ins(Instruction::Call(register)); // -> i32 token
				// Keep a copy of the token: the host call consumes it, but we also
				// return it as a Pluma `int` (a handler id the diff uses to overwrite
				// the slot in place via `dom.set-handler`).
				let tok = self.fresh_local(ValType::I32);
				self.ins(Instruction::LocalTee(tok));
				self.ins(Instruction::Call(idx));
				self.ins(Instruction::LocalGet(tok));
				self.ins(Instruction::I64ExtendI32S);
				self.box_int();
			}
			// `(externref, externref) -> ()` — `dom-remove-child` (same as `Append`).
			DomKind::Append2 => {
				self.unbox_extern(&args[0]);
				self.unbox_extern(&args[1]);
				self.ins(Instruction::Call(idx));
				self.push_nothing();
			}
			// `(externref, externref, externref) -> ()` — `replace-child`/`insert-before`.
			DomKind::Extern3 => {
				self.unbox_extern(&args[0]);
				self.unbox_extern(&args[1]);
				self.unbox_extern(&args[2]);
				self.ins(Instruction::Call(idx));
				self.push_nothing();
			}
			// `(externref, ptr, len) -> ()` — `remove-attribute` (node + one string).
			DomKind::NodeStr => {
				let (Some(alloc), Some(store)) = (alloc, store) else {
					self
						.diags
						.push(format!("`{tag}` needs the marshalling helpers"));
					self.push_nothing();
					return;
				};
				self.reset_bump();
				let (ptr, len) = self.marshal_strlike_arg(&args[1], alloc, store);
				self.unbox_extern(&args[0]);
				self.ins(Instruction::LocalGet(ptr));
				self.ins(Instruction::LocalGet(len));
				self.ins(Instruction::Call(idx));
				self.push_nothing();
			}
			// `(externref) -> ()` — `event-prevent-default`.
			DomKind::Extern1 => {
				self.unbox_extern(&args[0]);
				self.ins(Instruction::Call(idx));
				self.push_nothing();
			}
			// `(externref, i32) -> externref` — `dom-child-at`: node + index in, box the
			// returned child node. TAG first (bottom of stack), so after the call the stack
			// is `[TAG_EXTERN, childref]` for the `StructNew`.
			DomKind::ChildAt => {
				self.ins(Instruction::I32Const(types::TAG_EXTERN));
				self.unbox_extern(&args[0]);
				self.atom(&args[1]);
				self.unbox_int(); // int box -> i64
				self.ins(Instruction::I32WrapI64); // -> i32 index
				self.ins(Instruction::Call(idx));
				self.ins(Instruction::StructNew(types::T_EXTERN));
			}
			// `(kp, kl, vp, vl) -> ()` — the dev store write: two scratch strings, no node.
			DomKind::DevStoreSet => {
				let (Some(alloc), Some(store)) = (alloc, store) else {
					self
						.diags
						.push(format!("`{tag}` needs the marshalling helpers"));
					self.push_nothing();
					return;
				};
				self.reset_bump();
				let (kp, kl) = self.marshal_strlike_arg(&args[0], alloc, store);
				let (vp, vl) = self.marshal_strlike_arg(&args[1], alloc, store);
				self.ins(Instruction::LocalGet(kp));
				self.ins(Instruction::LocalGet(kl));
				self.ins(Instruction::LocalGet(vp));
				self.ins(Instruction::LocalGet(vl));
				self.ins(Instruction::Call(idx));
				self.push_nothing();
			}
			// `(kp, kl, dst, cap) -> len` — the dev store read.
			DomKind::DevStoreGet => {
				let (Some(alloc), Some(store), Some(load)) = (alloc, store, load) else {
					self
						.diags
						.push(format!("`{tag}` needs the marshalling helpers"));
					self.push_nothing();
					return;
				};
				self.emit_dom_dev_store_get(idx, &args[0], alloc, store, load);
			}
		}
	}

	/// `dom.dev-store-get key`: `(kp, kl, dst, cap) -> len` — marshal the key string into
	/// scratch, then have the host write the stored value into a fresh scratch region at
	/// `dst`; build a `$str` from `(dst, len)`. Like `emit_dom_get_value` but with a string
	/// key instead of a node. A generous fixed `CAP` (a model larger than this loses HMR
	/// and falls back to `init` — fine for dev).
	fn emit_dom_dev_store_get(&mut self, idx: u32, key: &Atom, alloc: u32, store: u32, load: u32) {
		const CAP: i32 = 1 << 16;
		self.reset_bump();
		// Key first, so its scratch region is distinct from the value buffer below.
		let (kp, kl) = self.marshal_strlike_arg(key, alloc, store);
		let dst = self.fresh_local(ValType::I32);
		self.ins(Instruction::I32Const(CAP));
		self.ins(Instruction::Call(alloc));
		self.ins(Instruction::LocalSet(dst));
		self.ins(Instruction::LocalGet(kp));
		self.ins(Instruction::LocalGet(kl));
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::I32Const(CAP));
		self.ins(Instruction::Call(idx)); // -> i32 len
		let len = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalSet(len));
		// Clamp to CAP (the host writes ≤ CAP bytes; an over-long value reads as a
		// truncated — hence undecodable — string, which falls back to `init`).
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::I32Const(CAP));
		self.ins(Instruction::I32GtS);
		self.ins(Instruction::If(BlockType::Empty));
		self.ins(Instruction::I32Const(CAP));
		self.ins(Instruction::LocalSet(len));
		self.ins(Instruction::End);
		self.ins(Instruction::I32Const(types::TAG_STR));
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::Call(load));
		self.ins(Instruction::StructNew(types::T_STR));
	}

	/// `dom.get-value node`: `(externref, dst, cap) -> len` — the host writes the node's
	/// `.value` into scratch at `dst`; build a `$str` from `(dst, len)`. A fixed `CAP`,
	/// no overflow drain (input values are short; a longer value clamps to `CAP`).
	fn emit_dom_get_value(&mut self, idx: u32, node: &Atom, alloc: u32, load: u32) {
		const CAP: i32 = 4096;
		self.reset_bump();
		let dst = self.fresh_local(ValType::I32);
		self.ins(Instruction::I32Const(CAP));
		self.ins(Instruction::Call(alloc));
		self.ins(Instruction::LocalSet(dst));
		self.unbox_extern(node);
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::I32Const(CAP));
		self.ins(Instruction::Call(idx)); // -> i32 len
		let len = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalSet(len));
		// Clamp the length to CAP (the host writes ≤ CAP bytes into scratch).
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::I32Const(CAP));
		self.ins(Instruction::I32GtS);
		self.ins(Instruction::If(BlockType::Empty));
		self.ins(Instruction::I32Const(CAP));
		self.ins(Instruction::LocalSet(len));
		self.ins(Instruction::End);
		self.ins(Instruction::I32Const(types::TAG_STR));
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::Call(load));
		self.ins(Instruction::StructNew(types::T_STR));
	}

	fn emit_io(&mut self, tag: &str, idx: u32, args: &[Atom]) {
		let kind = io_kind(tag).expect("emit_io on a non-io tag");
		let (Some(alloc), Some(store)) = (
			self.runtime.idx(Helper::MarshalAlloc),
			self.runtime.idx(Helper::MarshalStore),
		) else {
			self
				.diags
				.push(format!("`{tag}` needs the marshalling helpers"));
			self.push_nothing();
			return;
		};
		// Initial read buffer cap: most lines/files/dir-listings fit, so the common
		// path is a single host call; a larger read takes the overflow branch.
		const READ_CAP: i32 = 4096;
		self.reset_bump();
		match kind {
			IoKind::PathBool => {
				let (pp, pl) = self.marshal_strlike_arg(&args[0], alloc, store);
				self.ins(Instruction::LocalGet(pp));
				self.ins(Instruction::LocalGet(pl));
				self.ins(Instruction::Call(idx)); // -> i32 bool
				let b = self.fresh_local(ValType::I32);
				self.ins(Instruction::LocalSet(b));
				self.ins(Instruction::I32Const(types::TAG_BOOL));
				self.ins(Instruction::LocalGet(b));
				self.ins(Instruction::StructNew(types::T_BOOL));
			}
			IoKind::PathStatus | IoKind::WriteFile => {
				let (pp, pl) = self.marshal_strlike_arg(&args[0], alloc, store);
				self.ins(Instruction::LocalGet(pp));
				self.ins(Instruction::LocalGet(pl));
				if kind == IoKind::WriteFile {
					let (dp, dl) = self.marshal_strlike_arg(&args[1], alloc, store);
					self.ins(Instruction::LocalGet(dp));
					self.ins(Instruction::LocalGet(dl));
				}
				self.ins(Instruction::Call(idx)); // -> i32 status
				self.shape_io_status(tag);
			}
			IoKind::ReadStr
			| IoKind::ReadBytes
			| IoKind::ReadFileStr
			| IoKind::ReadFileBytes
			| IoKind::ReadDir => {
				// Path reads encode the path arg first; the unit-arg reads ignore it.
				let path = matches!(
					kind,
					IoKind::ReadFileStr | IoKind::ReadFileBytes | IoKind::ReadDir
				)
				.then(|| self.marshal_strlike_arg(&args[0], alloc, store));
				self.emit_io_read(tag, idx, path, kind, READ_CAP, alloc);
			}
			IoKind::Args => {
				// No path arg (the `()` unit is ignored); returns a bare `list string`.
				self.emit_io_args(tag, idx, READ_CAP, alloc);
			}
			IoKind::EnvVar => {
				// Marshal the var name like a path read; result is an `option string`.
				let name = self.marshal_strlike_arg(&args[0], alloc, store);
				self.emit_env(tag, idx, name, READ_CAP, alloc);
			}
			IoKind::FsOpSync => {
				self.emit_fs_op_sync(tag, idx, args, READ_CAP, alloc, store);
			}
		}
	}

	/// `fs-op-sync(op, pp, pl, dp, dl, dst, cap) -> len`: the synchronous `std/sys/fs` op.
	/// Marshal the op-code + path + data into scratch, length-probe the bytes result (with
	/// the `io-copyout` overflow drain), and wrap it `ok`/`err` via `__io_result`. Like a
	/// path read, but with the leading op-code i32 and a second (data) string arg. The
	/// Pluma `-sync` wrapper decodes the bytes per op (`args` = [op, path, data]).
	fn emit_fs_op_sync(
		&mut self,
		tag: &str,
		idx: u32,
		args: &[Atom],
		cap: i32,
		alloc: u32,
		store: u32,
	) {
		let (Some(load), Some(io_result)) = (
			self.runtime.idx(Helper::MarshalLoad),
			self.runtime.idx(Helper::IoResult),
		) else {
			self
				.diags
				.push(format!("`{tag}` needs the io read helpers"));
			self.push_nothing();
			return;
		};
		let Some(copyout) = self.host_index.get("io-copyout").copied() else {
			self
				.diags
				.push(format!("`{tag}` needs the io-copyout import"));
			self.push_nothing();
			return;
		};
		// op-code (i32), then the path + data strings into scratch.
		self.atom(&args[0]);
		self.unbox_int();
		self.ins(Instruction::I32WrapI64);
		let op = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalSet(op));
		let (pp, pl) = self.marshal_strlike_arg(&args[1], alloc, store);
		let (dp, dl) = self.marshal_strlike_arg(&args[2], alloc, store);
		let dst = self.fresh_local(ValType::I32);
		self.ins(Instruction::I32Const(cap));
		self.ins(Instruction::Call(alloc));
		self.ins(Instruction::LocalSet(dst));
		// fs-op-sync(op, pp, pl, dp, dl, dst, cap) -> len.
		for v in [op, pp, pl, dp, dl, dst] {
			self.ins(Instruction::LocalGet(v));
		}
		self.ins(Instruction::I32Const(cap));
		self.ins(Instruction::Call(idx));
		let len = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalSet(len));
		// Overflow: the bytes didn't fit `cap` — reserve the true size, drain the stash.
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::I32Const(cap));
		self.ins(Instruction::I32GtS);
		self.ins(Instruction::If(BlockType::Empty));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::Call(alloc));
		self.ins(Instruction::LocalSet(dst));
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::Call(copyout));
		self.ins(Instruction::End);
		// payload-or-null: `len < 0` is failure (null → `__io_result` builds `err`).
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::I32Const(0));
		self.ins(Instruction::I32LtS);
		self.ins(Instruction::If(BlockType::Result(types::value_ref())));
		self.push_nothing();
		self.ins(Instruction::Else);
		self.ins(Instruction::I32Const(types::TAG_BYTES));
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::Call(load));
		self.ins(Instruction::StructNew(types::T_STR));
		self.ins(Instruction::End);
		self.ins(Instruction::Call(io_result));
	}

	/// The length-probe read core: `dst = __alloc(cap)`; call the import (`(dst, cap)`
	/// or `(path, plen, dst, cap)`) → `len`; on `len > cap` (overflow) re-`__alloc`
	/// the true size and `__io_copyout` the host's stash; then build the payload from
	/// `(dst, len)` (or null when `len < 0`) and wrap it via `__io_result`.
	fn emit_io_read(
		&mut self,
		tag: &str,
		idx: u32,
		path: Option<(u32, u32)>,
		kind: IoKind,
		cap: i32,
		alloc: u32,
	) {
		let (Some(load), Some(io_result)) = (
			self.runtime.idx(Helper::MarshalLoad),
			self.runtime.idx(Helper::IoResult),
		) else {
			self
				.diags
				.push(format!("`{tag}` needs the io read helpers"));
			self.push_nothing();
			return;
		};
		let Some((dst, len)) = self.emit_io_probe(tag, idx, path, cap, alloc) else {
			return;
		};
		// payload-or-null: `len < 0` is failure (null → `__io_result` builds `err`).
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::I32Const(0));
		self.ins(Instruction::I32LtS);
		self.ins(Instruction::If(BlockType::Result(types::value_ref())));
		self.push_nothing();
		self.ins(Instruction::Else);
		match kind {
			IoKind::ReadDir => {
				let read_names = self
					.runtime
					.idx(Helper::MarshalReadNames)
					.expect("read-dir needs __read_names");
				self.ins(Instruction::LocalGet(dst));
				self.ins(Instruction::LocalGet(len));
				self.ins(Instruction::Call(read_names));
			}
			_ => {
				// `$str` / `$bytes` share the `{tag, $bytes}` struct; only the tag differs.
				let tag_const = match kind {
					IoKind::ReadBytes | IoKind::ReadFileBytes => types::TAG_BYTES,
					_ => types::TAG_STR,
				};
				self.ins(Instruction::I32Const(tag_const));
				self.ins(Instruction::LocalGet(dst));
				self.ins(Instruction::LocalGet(len));
				self.ins(Instruction::Call(load));
				self.ins(Instruction::StructNew(types::T_STR));
			}
		}
		self.ins(Instruction::End);
		self.ins(Instruction::Call(io_result));
	}

	/// The length-probe read core, shared by the `result`-shaped reads (`emit_io_read`)
	/// and the bare-list `io.args` read (`emit_io_args`): `dst = __alloc(cap)`; call the
	/// import (`(dst, cap)` or `(path, plen, dst, cap)`) → `len`; on `len > cap`
	/// (overflow) re-`__alloc` the true size and `__io_copyout` the host's stash. Returns
	/// the `(dst, len)` locals, or `None` (after pushing `nothing` + a diag) if the
	/// `io-copyout` import is missing.
	fn emit_io_probe(
		&mut self,
		tag: &str,
		idx: u32,
		path: Option<(u32, u32)>,
		cap: i32,
		alloc: u32,
	) -> Option<(u32, u32)> {
		let copyout = match self.host_index.get("io-copyout").copied() {
			Some(c) => c,
			None => {
				self
					.diags
					.push(format!("`{tag}` needs the io-copyout import"));
				self.push_nothing();
				return None;
			}
		};
		let dst = self.fresh_local(ValType::I32);
		self.ins(Instruction::I32Const(cap));
		self.ins(Instruction::Call(alloc));
		self.ins(Instruction::LocalSet(dst));
		// Call: io4 reads pass (path, plen) first; io2 reads pass just (dst, cap).
		if let Some((pp, pl)) = path {
			self.ins(Instruction::LocalGet(pp));
			self.ins(Instruction::LocalGet(pl));
		}
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::I32Const(cap));
		self.ins(Instruction::Call(idx)); // -> i32 len
		let len = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalSet(len));
		// Overflow: the bytes didn't fit `cap` — reserve the true size, drain the stash.
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::I32Const(cap));
		self.ins(Instruction::I32GtS);
		self.ins(Instruction::If(BlockType::Empty));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::Call(alloc));
		self.ins(Instruction::LocalSet(dst));
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::Call(copyout));
		self.ins(Instruction::End);
		Some((dst, len))
	}

	/// `io.args`: length-probe the argv blob (NUL-terminated names in scratch), then split
	/// it into a bare `$list` of `$str` via `__read_names`. No `__io_result` wrap — argv
	/// never fails, and an empty argv (`len == 0`) is the empty list.
	fn emit_io_args(&mut self, tag: &str, idx: u32, cap: i32, alloc: u32) {
		let Some(read_names) = self.runtime.idx(Helper::MarshalReadNames) else {
			self.diags.push(format!("`{tag}` needs __read_names"));
			self.push_nothing();
			return;
		};
		let Some((dst, len)) = self.emit_io_probe(tag, idx, None, cap, alloc) else {
			return;
		};
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::Call(read_names));
	}

	/// `io.env`: marshal the var name (already done by the caller), length-probe the
	/// value, then shape `len` into an `option string` — `len < 0` (unset) → `none`,
	/// else `some ($str of the value)`. Builds the `some`/`none` `$variant`s inline from
	/// `runtime.opt` (the option enum's tags + names), so there's no `__io_result` and
	/// the host stays out of the enum layout.
	fn emit_env(&mut self, tag: &str, idx: u32, name: (u32, u32), cap: i32, alloc: u32) {
		let Some(load) = self.runtime.idx(Helper::MarshalLoad) else {
			self.diags.push(format!("`{tag}` needs __load_bytes"));
			self.push_nothing();
			return;
		};
		let opt = self.runtime.opt;
		let Some((dst, len)) = self.emit_io_probe(tag, idx, Some(name), cap, alloc) else {
			return;
		};
		// len < 0 (unset) -> none ; else some($str(__load_bytes(dst, len))).
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::I32Const(0));
		self.ins(Instruction::I32LtS);
		self.ins(Instruction::If(BlockType::Result(types::value_ref())));
		// none: `$variant { TAG_VARIANT, none_tag, none_name, arity 0, …null }`.
		self.ins(Instruction::I32Const(types::TAG_VARIANT));
		self.ins(Instruction::I32Const(opt.none_tag as i32));
		self.str_seg(opt.none_name);
		self.ins(Instruction::I32Const(0));
		self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
		self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
		self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALARRAY)));
		self.ins(Instruction::StructNew(types::T_VARIANT));
		self.ins(Instruction::Else);
		// some value: `$variant { …, some_name, arity 1, p0 = $str(value), …null }`.
		self.ins(Instruction::I32Const(types::TAG_VARIANT));
		self.ins(Instruction::I32Const(opt.some_tag as i32));
		self.str_seg(opt.some_name);
		self.ins(Instruction::I32Const(1));
		self.ins(Instruction::I32Const(types::TAG_STR));
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::Call(load));
		self.ins(Instruction::StructNew(types::T_STR));
		self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
		self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALARRAY)));
		self.ins(Instruction::StructNew(types::T_VARIANT));
		self.ins(Instruction::End);
	}

	/// Push a `$str` for an interned data-segment string `(off, len)` — like
	/// `string_const`, but from a pre-interned segment ref (e.g. an `option` variant
	/// name carried in `runtime.opt`).
	fn str_seg(&mut self, (off, len): (u32, u32)) {
		self.ins(Instruction::I32Const(types::TAG_STR));
		self.ins(Instruction::I32Const(off as i32));
		self.ins(Instruction::I32Const(len as i32));
		self.ins(Instruction::ArrayNewData {
			array_type_index: types::T_BYTES,
			array_data_index: 0,
		});
		self.ins(Instruction::StructNew(types::T_STR));
	}

	/// `std/time` clock host imports. now/monotonic box the host's i64 under
	/// `TAG_INSTANT`/`TAG_DURATION` (both reuse the `$int` `{tag, i64}` shape); sleep
	/// unboxes its `duration` arg to i64 and calls the blocking import; parse marshals
	/// two strings + a scratch i64 slot into a `result instant string`.
	fn emit_clock(&mut self, tag: &str, idx: u32, args: &[Atom]) {
		match clock_kind(tag) {
			Some(ClockKind::NowInstant) => self.emit_clock_scalar(idx, types::TAG_INSTANT),
			Some(ClockKind::MonotonicDuration) => self.emit_clock_scalar(idx, types::TAG_DURATION),
			Some(ClockKind::Sleep) => {
				// Unbox the `duration` arg to i64, block, then materialize `nothing`.
				self.atom(&args[0]);
				self.unbox_int();
				self.ins(Instruction::Call(idx)); // (i64) -> ()
				self.push_nothing();
			}
			Some(ClockKind::Parse) => self.emit_time_parse(tag, idx, args),
			None => {
				self
					.diags
					.push(format!("emit_clock on non-clock tag `{tag}`"));
				self.push_nothing();
			}
		}
	}

	/// `time.now`/`time.monotonic`: call the `() -> i64` import and box the result under
	/// `tag_const` (`TAG_INSTANT`/`TAG_DURATION`, both the `$int` `{tag, i64}` layout).
	/// The `()` unit arg is dropped, like `random.int`.
	fn emit_clock_scalar(&mut self, idx: u32, tag_const: i32) {
		self.ins(Instruction::I32Const(tag_const));
		self.ins(Instruction::Call(idx)); // () -> i64
		self.ins(Instruction::StructNew(types::T_INT));
	}

	/// `time.parse fmt input`: marshal both strings + an 8-byte scratch slot, call
	/// `time-parse(fp, fl, ip, il, dst) -> status`; on `status == 0` read the i64 nanos
	/// back (`i64.load`) and box an `instant` payload, else push null — `__io_result`
	/// then wraps `ok (instant …)` / `err (io-last-error())`.
	fn emit_time_parse(&mut self, tag: &str, idx: u32, args: &[Atom]) {
		let (Some(alloc), Some(store), Some(io_result)) = (
			self.runtime.idx(Helper::MarshalAlloc),
			self.runtime.idx(Helper::MarshalStore),
			self.runtime.idx(Helper::IoResult),
		) else {
			self
				.diags
				.push(format!("`{tag}` needs the marshalling helpers"));
			self.push_nothing();
			return;
		};
		self.reset_bump();
		let (fp, fl) = self.marshal_strlike_arg(&args[0], alloc, store);
		let (ip, il) = self.marshal_strlike_arg(&args[1], alloc, store);
		let dst = self.fresh_local(ValType::I32);
		self.ins(Instruction::I32Const(8));
		self.ins(Instruction::Call(alloc));
		self.ins(Instruction::LocalSet(dst));
		self.ins(Instruction::LocalGet(fp));
		self.ins(Instruction::LocalGet(fl));
		self.ins(Instruction::LocalGet(ip));
		self.ins(Instruction::LocalGet(il));
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::Call(idx)); // -> i32 status
		// status == 0 -> ok (instant load(dst)); else null -> err (io-last-error).
		self.ins(Instruction::I32Eqz);
		self.ins(Instruction::If(BlockType::Result(types::value_ref())));
		self.ins(Instruction::I32Const(types::TAG_INSTANT));
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::I64Load(MemArg {
			offset: 0,
			align: 0,
			memory_index: 0,
		}));
		self.ins(Instruction::StructNew(types::T_INT));
		self.ins(Instruction::Else);
		self.push_nothing();
		self.ins(Instruction::End);
		self.ins(Instruction::Call(io_result));
	}

	/// Shape a writer/`mkdir`/`delete` `i32 status` into the value `__io_result`
	/// wraps: `0` → a heap `nothing` (`ok nothing`), non-zero → null (`err`).
	fn shape_io_status(&mut self, tag: &str) {
		let Some(io_result) = self.runtime.idx(Helper::IoResult) else {
			self
				.diags
				.push(format!("`{tag}` needs the __io_result shaper"));
			self.push_nothing();
			return;
		};
		self.ins(Instruction::I32Eqz); // status == 0 (ok)?
		self.ins(Instruction::If(BlockType::Result(types::value_ref())));
		// A successful `nothing` must be non-null (null = failure to `__io_result`).
		self.ins(Instruction::I32Const(types::TAG_NOTHING));
		self.ins(Instruction::StructNew(types::T_VALUE));
		self.ins(Instruction::Else);
		self.push_nothing();
		self.ins(Instruction::End);
		self.ins(Instruction::Call(io_result));
	}

	/// `debug x`: print `[<module>:<line>] <to-string x>` (the host `print` import
	/// appends the newline), then leave `x` on the stack unchanged. The
	/// `<module>:<line>` call site is known statically, so the
	/// prefix is built inline (an `array.new_fixed` of its bytes) and the value's
	/// `to-string` bytes are concatenated onto it. The atom is re-emitted as the
	/// rvalue; atoms (a var or inline const) are side-effect-free to evaluate twice.
	fn emit_debug(&mut self, args: &[Atom]) {
		let arg = &args[0];
		let prefix = format!("[{}:{}] ", self.f.module, self.cur_line + 1);
		let (Some(ts), Some(bc)) = (
			self.runtime.idx(Helper::ToString),
			self.runtime.idx(Helper::BytesConcat),
		) else {
			self
				.diags
				.push("debug needs __tostring/__bytesconcat".to_string());
			self.atom(arg);
			return;
		};
		// Prefix bytes (compile-time constant) as a `$bytes` array.
		for &b in prefix.as_bytes() {
			self.ins(Instruction::I32Const(b as i32));
		}
		self.ins(Instruction::ArrayNewFixed {
			array_type_index: types::T_BYTES,
			array_size: prefix.len() as u32,
		});
		// `to-string(arg)` -> `$str`; take its backing `$bytes`.
		self.atom(arg);
		self.ins(Instruction::Call(ts));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_STR,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_STR,
			field_index: 1,
		});
		// Concat (prefix ++ rendered value) into the line `$bytes`.
		self.ins(Instruction::Call(bc));
		// Marshal the line into scratch and print it (host appends the newline).
		let (Some(send), Some(print_idx)) = (
			self.runtime.idx(Helper::MarshalSend),
			self.host_index.get("print").copied(),
		) else {
			self
				.diags
				.push("debug needs __send_bytes + the `print` host import".to_string());
			self.atom(arg);
			return;
		};
		self.ins(Instruction::Call(send)); // ($bytes) -> len
		let len = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalSet(len));
		self.ins(Instruction::I32Const(0));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::Call(print_idx));
		// `debug` returns its argument unchanged.
		self.atom(arg);
	}

	/// Shape a synchronous `std/sys/net` op into a `result` over the marshalling ABI.
	/// `listen`/`connect` encode the address into scratch and call `(addr, alen) ->
	/// (status, id)`; `close` passes the unboxed socket id and calls `(id) -> status`;
	/// `local-addr` length-probes the address string into scratch. Each shapes the
	/// `(status, …)` return through `__io_result` (status 0 → `ok …`; non-zero → null
	/// → `err (io-last-error())`, the message set host-side). `connect`'s static type
	/// is `task (result …)`, so its `result` is wrapped in a Pure `$task`.
	fn emit_net_sync(&mut self, tag: &str, args: &[Atom]) {
		let (Some(net), Some(alloc), Some(store), Some(io_result)) = (
			self.runtime.net,
			self.runtime.idx(Helper::MarshalAlloc),
			self.runtime.idx(Helper::MarshalStore),
			self.runtime.idx(Helper::IoResult),
		) else {
			self
				.diags
				.push(format!("`{tag}` needs the net marshalling helpers"));
			self.push_nothing();
			return;
		};
		// Address strings are short; the length-probe cap never overflows.
		const ADDR_CAP: i32 = 256;
		self.reset_bump();
		match tag {
			"net-listen" => {
				// (addr, alen) -> (status, socket-id). (connect is no longer here — it's an
				// offloaded suspending op, built via `make_task` like accept/read/write.)
				let (ap, al) = self.marshal_strlike_arg(&args[0], alloc, store);
				self.ins(Instruction::LocalGet(ap));
				self.ins(Instruction::LocalGet(al));
				self.ins(Instruction::Call(net.listen));
				self.shape_net_id_result(io_result);
			}
			"net-close" => {
				// (id) -> status; reuse the io status → `ok nothing` / `err` shaper.
				self.atom(&args[0]);
				self.unbox_int();
				self.ins(Instruction::I32WrapI64);
				self.ins(Instruction::Call(net.close));
				self.shape_io_status(tag);
			}
			"net-local-addr" => {
				// (id, dst, cap) -> (status, len); ok payload = the address `$str`.
				let id = self.fresh_local(ValType::I32);
				self.atom(&args[0]);
				self.unbox_int();
				self.ins(Instruction::I32WrapI64);
				self.ins(Instruction::LocalSet(id));
				let dst = self.fresh_local(ValType::I32);
				self.ins(Instruction::I32Const(ADDR_CAP));
				self.ins(Instruction::Call(alloc));
				self.ins(Instruction::LocalSet(dst));
				self.ins(Instruction::LocalGet(id));
				self.ins(Instruction::LocalGet(dst));
				self.ins(Instruction::I32Const(ADDR_CAP));
				self.ins(Instruction::Call(net.local_addr));
				self.shape_net_str_result(io_result, dst);
			}
			other => {
				self.diags.push(format!("`{other}` is not a sync net op"));
				self.push_nothing();
			}
		}
	}

	/// `web-fetch req` (the browser HTTP transport under the *sys* host, `std/web/fetch`):
	/// marshal the request string into scratch, call the io-read-shaped `(req_ptr,
	/// req_len, dst, cap) -> len` host import (the host runs the exchange and writes the
	/// reply, `len < 0` = failure, an overflow draining via `__io_copyout`), shape the
	/// reply `$str` through `__io_result` (`ok reply` / `err (io-last-error())`), and wrap
	/// it in a Pure `$task` to match the builtin's `task (result string string)` type. A
	/// browser build routes `web-fetch` to the async `WebFetch` helper instead.
	fn emit_web_fetch(&mut self, idx: u32, args: &[Atom]) {
		let (Some(alloc), Some(store)) = (
			self.runtime.idx(Helper::MarshalAlloc),
			self.runtime.idx(Helper::MarshalStore),
		) else {
			self
				.diags
				.push("`web-fetch` needs the marshalling helpers".to_string());
			self.push_nothing();
			return;
		};
		// Responses are typically small; start with a generous cap and let the io-read
		// overflow drain handle anything bigger.
		const CAP: i32 = 1 << 16;
		self.reset_bump();
		let (rp, rl) = self.marshal_strlike_arg(&args[0], alloc, store);
		// Reuse the io read path: `(req_ptr, req_len, dst, cap) -> len`; it probes,
		// drains overflow, builds the `$str`, and wraps it in `ok`/`err` via `__io_result`.
		self.emit_io_read(
			"web-fetch",
			idx,
			Some((rp, rl)),
			IoKind::ReadFileStr,
			CAP,
			alloc,
		);
		// Wrap the `result` (on the stack) in `task.return result` -- a Pure `$task`.
		let r = self.fresh_local(types::value_ref());
		self.ins(Instruction::LocalSet(r));
		self.ins(Instruction::I32Const(types::TAG_TASK));
		self.ins(Instruction::I32Const(task_kind::PURE));
		self.ins(Instruction::LocalGet(r));
		self.ins(Instruction::ArrayNewFixed {
			array_type_index: types::T_VALARRAY,
			array_size: 1,
		});
		self.ins(Instruction::StructNew(types::T_TASK));
	}

	/// Shape a net op's `(status, socket-id)` return (on the stack) into a `result`:
	/// status 0 → `ok <boxed id>`, non-zero → null → `err (io-last-error())`.
	fn shape_net_id_result(&mut self, io_result: u32) {
		let id = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalSet(id));
		self.ins(Instruction::I32Eqz); // status == 0 (ok)?
		self.ins(Instruction::If(BlockType::Result(types::value_ref())));
		self.ins(Instruction::I32Const(types::TAG_INT));
		self.ins(Instruction::LocalGet(id));
		self.ins(Instruction::I64ExtendI32S);
		self.ins(Instruction::StructNew(types::T_INT));
		self.ins(Instruction::Else);
		self.push_nothing();
		self.ins(Instruction::End);
		self.ins(Instruction::Call(io_result));
	}

	/// Shape a net op's `(status, len)` return into a `result` whose ok payload is the
	/// `$str` read out of scratch at `dst`: status 0 → `ok <str>`, non-zero → `err`.
	fn shape_net_str_result(&mut self, io_result: u32, dst: u32) {
		let load = self
			.runtime
			.idx(Helper::MarshalLoad)
			.expect("net-local-addr needs __load_bytes");
		let len = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalSet(len));
		self.ins(Instruction::I32Eqz); // status == 0 (ok)?
		self.ins(Instruction::If(BlockType::Result(types::value_ref())));
		self.ins(Instruction::I32Const(types::TAG_STR));
		self.ins(Instruction::LocalGet(dst));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::Call(load));
		self.ins(Instruction::StructNew(types::T_STR));
		self.ins(Instruction::Else);
		self.push_nothing();
		self.ins(Instruction::End);
		self.ins(Instruction::Call(io_result));
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
				self.unbox_int();
				self.ins(Instruction::I32WrapI64);
				self.ins(Instruction::ArrayGet(types::T_VALARRAY));
			}
			// dom.set-handler token closure : overwrite the handler-registry slot at
			// `token` in place, so the diff can re-point a reused node's listener
			// without detaching it. Same `array.set` shape as `list-set`, but the
			// target array is the `dom_handlers` registry global (browser-only; the
			// global is non-null here because a token only exists once `add-listener`
			// has run and lazily created it).
			"dom-set-handler" => {
				let g = self
					.runtime
					.dom_handlers
					.expect("dom-set-handler needs the dom_handlers global");
				self.ins(Instruction::GlobalGet(g));
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 1,
				});
				self.atom(&args[0]);
				self.unbox_int();
				self.ins(Instruction::I32WrapI64);
				self.atom(&args[1]);
				self.ins(Instruction::ArraySet(types::T_VALARRAY));
				self.push_nothing();
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
				self.unbox_int();
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
			// local.new default : a fresh `$local` cell carrying its default value.
			// The struct reference is the cell's identity (`ref.eq`); the binding env
			// keyed off it lives per-fiber (see `helpers/task.rs`).
			"local-new" => {
				self.ins(Instruction::I32Const(types::TAG_LOCAL));
				self.atom(&args[0]);
				self.ins(Instruction::StructNew(types::T_LOCAL));
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
			// list.length xs : element count (the logical `length` field, field 2 —
			// NOT array.len of the backing array, which is the capacity), boxed `$int`.
			"list-length" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
					types::T_LIST,
				)));
				self.ins(Instruction::StructGet {
					struct_type_index: types::T_LIST,
					field_index: 2,
				});
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
				self.unbox_int();
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
			// the IEEE-754 result of `f64::sqrt`.
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
			// trunc has `f as i64` semantics (NaN -> 0, ±inf / out-of-range
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
				self.unbox_int();
				self.ins(Instruction::F64ConvertI64S);
				self.ins(Instruction::StructNew(types::T_FLOAT));
			}
			// bit.and/or/xor a b : one i64 logical opcode over the unboxed payloads.
			"bit-and" => self.int_binop(args, Instruction::I64And),
			"bit-or" => self.int_binop(args, Instruction::I64Or),
			"bit-xor" => self.int_binop(args, Instruction::I64Xor),
			// bit.shift-* a n : `n` is the shift count (wasm takes it mod 64). `shr_s`
			// preserves the sign bit (arithmetic), `shr_u` fills zeros (logical).
			"bit-shift-left" => self.int_binop(args, Instruction::I64Shl),
			"bit-shift-right" => self.int_binop(args, Instruction::I64ShrS),
			"bit-shift-right-unsigned" => self.int_binop(args, Instruction::I64ShrU),
			// bit.not a : flip every bit. WasmGC has no i64 `not`, so it's `a xor -1`.
			"bit-not" => {
				self.ins(Instruction::I32Const(types::TAG_INT));
				self.atom(&args[0]);
				self.unbox_int();
				self.ins(Instruction::I64Const(-1));
				self.ins(Instruction::I64Xor);
				self.ins(Instruction::StructNew(types::T_INT));
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

	/// A `bit.*` binary integer op: push `TAG_INT`, unbox both `$int` payloads to
	/// i64, apply the single integer opcode, and rebox as `$int`. The shift ops
	/// reuse this — wasm shifts take the count (second operand) mod 64.
	fn int_binop(&mut self, args: &[Atom], op: Instruction<'static>) {
		self.ins(Instruction::I32Const(types::TAG_INT));
		self.atom(&args[0]);
		self.unbox_int();
		self.atom(&args[1]);
		self.unbox_int();
		self.ins(op);
		self.ins(Instruction::StructNew(types::T_INT));
	}

	/// Retag an `$int`-shaped box (`{tag, i64}`) under `new_tag`: read the i64
	/// payload out and rebox it. The `duration` and `instant` carriers share the
	/// `$int` struct shape and differ only by tag, so box/unbox between them and a
	/// plain `int` is just this retag.
	fn retag_int_box(&mut self, arg: &Atom, new_tag: i32) {
		self.ins(Instruction::I32Const(new_tag));
		self.atom(arg);
		// `arg` may be a plain `int` (an `i31ref` when small) being retagged to a
		// duration/instant, so read it through `unbox_int`.
		self.unbox_int();
		self.ins(Instruction::StructNew(types::T_INT));
	}

	/// Push an atom as a uniform boxed `$value`. A var holding a *nominal* record
	/// (`$shapeN`) is `lift`ed to the uniform `$record` here, so every consumer that
	/// isn't a record read (a call arg, a container element, a `Return`, a stored
	/// field, a generic consumer) sees the self-describing representation it
	/// expects. Read sites (`GetField` receiver, `Match` subject) use `atom_raw`
	/// instead, keeping the `$shapeN` for a constant-index `struct.get`.
	fn atom(&mut self, a: &Atom) {
		// A nominal `$shapeN` is a `$value` subtype, so it flows raw everywhere a boxed
		// value goes — stored into a list/`$valarray`, passed as an arg, returned. It is
		// NOT lifted to the uniform `$record` here; the generic consumers that need a
		// uniform record (`__getfield`/`__eq`/`__tostring`/wire/`__hash`) self-lift it via
		// `__denominalize`. Keeping it nominal through containers is what lets a later
		// field read on it stay a constant-index `struct.get` once the reader is nominal.
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

	/// Build a *nominal* record: a `$shapeN` struct `{ tag, shape_id, f0..fk }` with
	/// the field values inline in the shape's name-sorted order. Field values are
	/// pushed via `atom` (so a nested nominal record is stored as the uniform
	/// `$record`, keeping field reads uniform). The result is a `(ref $shapeN)`,
	/// storable in a `(ref null $value)` local (it's a `$value` subtype).
	fn make_record_nominal(&mut self, shape: &ir::RecordShape, fields: &[(String, Atom)]) {
		let st = self.ftypes.intern_shape(&shape);
		let mut sorted: Vec<(&String, &Atom)> = fields.iter().map(|(n, a)| (n, a)).collect();
		sorted.sort_by(|a, b| a.0.cmp(b.0));
		self.ins(Instruction::I32Const(types::TAG_SHAPE));
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
		let st = self.ftypes.intern_shape(&shape);
		let base_local = match base {
			Atom::Var(v) => self.local(v.0),
			// A record base is always a var; fall back to the uniform path otherwise.
			Atom::Const(_) => {
				self.diags.push("record-update on a non-var nominal base");
				return;
			}
		};
		self.ins(Instruction::I32Const(types::TAG_SHAPE));
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
		// `nothing` is a null reference — no allocation. `value_tag` and the host map a
		// null value back to `()`; produced on essentially every statement.
		self.ins(Instruction::RefNull(HeapType::Concrete(types::T_VALUE)));
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
	/// Emit a `$list` from the `$valarray` currently on the stack top, setting the
	/// logical `length` field to the array's capacity. The list constructor (the
	/// 3-field `$list` struct); `list.push` is the only thing that later makes
	/// length < capacity.
	fn mk_list(&mut self) {
		let tmp = self.fresh_local(types::valarray_ref());
		self.ins(Instruction::LocalSet(tmp));
		self.ins(Instruction::I32Const(types::TAG_LIST));
		self.ins(Instruction::LocalGet(tmp));
		self.ins(Instruction::LocalGet(tmp));
		self.ins(Instruction::ArrayLen);
		self.ins(Instruction::StructNew(types::T_LIST));
	}

	/// The `$list` currently on the stack -> its elements as a `$valarray` of
	/// exactly `length` elements. When `length == capacity` (no `list.push` has
	/// grown it — the common case) this is the backing array itself, no copy;
	/// only a spare-capacity list is trimmed (so its tail never leaks into a
	/// spread / concat).
	fn emit_list_elems(&mut self) {
		let list_l = self.fresh_local(types::value_ref());
		self.ins(Instruction::LocalSet(list_l));
		let len = self.fresh_local(ValType::I32);
		self.ins(Instruction::LocalGet(list_l));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_LIST,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_LIST,
			field_index: 2,
		});
		self.ins(Instruction::LocalSet(len));
		let src = self.fresh_local(types::valarray_ref());
		self.ins(Instruction::LocalGet(list_l));
		self.ins(Instruction::RefCastNonNull(HeapType::Concrete(
			types::T_LIST,
		)));
		self.ins(Instruction::StructGet {
			struct_type_index: types::T_LIST,
			field_index: 1,
		});
		self.ins(Instruction::LocalSet(src));
		// if len == array.len(src): use src as-is (no spare capacity).
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::LocalGet(src));
		self.ins(Instruction::ArrayLen);
		self.ins(Instruction::I32Eq);
		self.ins(Instruction::If(BlockType::Result(types::valarray_ref())));
		self.ins(Instruction::LocalGet(src));
		self.ins(Instruction::Else);
		// trim: out[i] = src[i] for i in 0..len.
		let out = self.fresh_local(types::valarray_ref());
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::ArrayNewDefault(types::T_VALARRAY));
		self.ins(Instruction::LocalSet(out));
		let idx = self.fresh_local(ValType::I32);
		self.ins(Instruction::I32Const(0));
		self.ins(Instruction::LocalSet(idx));
		self.ins(Instruction::Block(BlockType::Empty));
		self.ins(Instruction::Loop(BlockType::Empty));
		self.ins(Instruction::LocalGet(idx));
		self.ins(Instruction::LocalGet(len));
		self.ins(Instruction::I32GeU);
		self.ins(Instruction::BrIf(1));
		self.ins(Instruction::LocalGet(out));
		self.ins(Instruction::LocalGet(idx));
		self.ins(Instruction::LocalGet(src));
		self.ins(Instruction::LocalGet(idx));
		self.ins(Instruction::ArrayGet(types::T_VALARRAY));
		self.ins(Instruction::ArraySet(types::T_VALARRAY));
		self.ins(Instruction::LocalGet(idx));
		self.ins(Instruction::I32Const(1));
		self.ins(Instruction::I32Add);
		self.ins(Instruction::LocalSet(idx));
		self.ins(Instruction::Br(0));
		self.ins(Instruction::End);
		self.ins(Instruction::End);
		self.ins(Instruction::LocalGet(out));
		self.ins(Instruction::End);
	}

	fn make_list(&mut self, items: &[ir::ListItem]) {
		use ir::ListItem;
		if !items.iter().any(|it| matches!(it, ListItem::Spread(_))) {
			self.elems_array(
				&items
					.iter()
					.map(|it| match it {
						ListItem::Elem(a) => a.clone(),
						ListItem::Spread(_) => unreachable!(),
					})
					.collect::<Vec<_>>(),
			);
			self.mk_list();
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
				self.emit_list_elems();
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
		self.mk_list();
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
