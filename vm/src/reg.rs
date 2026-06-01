//! Register-VM instruction set + program shape (M0/M1 — see
//! `notes/REGISTER_VM.md`).
//!
//! Defined alongside the stack VM during bring-up; M1 cuts the VM over to
//! execute this in place (the `drive_step` Await driver stays, now snapshotting
//! the register window; builtins are reused unchanged — `invoke` re-enters
//! register dispatch). `#![allow(dead_code)]` until that cutover wires it in.
//!
//! ## Model
//!
//! Each function owns a flat **register file** — `nregs` slots, addressed by
//! `Reg`. Registers subsume the stack VM's params + locals + operand stack:
//! every instruction names its sources and destination explicitly (three-
//! address form), so the `LoadLocal`/`StoreLocal` shuffle (28–49% of executed
//! opcodes on the stack VM) disappears. The register file lives in the unified
//! stack window `stack[base .. base + nregs]`, exactly like today's slots — the
//! frame model, frame cache, `Return`-truncation and tail-call reuse all carry
//! over.
//!
//! **Typed registers (M5+).** Per `reg_reprs`, a register is either a boxed
//! `Value` or a raw `i64`/`f64`/`i32`. The M0 microbench
//! (`tests/regfile_bench.rs`) settled the representation: **parallel arrays** —
//! a boxed `Vec<Value>` window beside a raw `Vec<u64>` window. Until M5 every
//! register is `Boxed` and the raw window is unused.
//!
//! ## Operand passing
//!
//! Multi-operand instructions (calls, constructors, interpolation, pattern
//! destructuring) name their operand registers through a **reg-list pool**
//! (`Program::reg_lists`, indexed by `RegListIdx`) rather than requiring the
//! operands to sit in a contiguous window. This keeps `Instruction` `Copy` and
//! lets naive allocation (one register per `VarId`, identity) emit calls without
//! gather-`Move`s — the VM marshals operands from the list, exactly as the stack
//! VM drains them off the stack today. A later pass (M2) can pin hot call args
//! to contiguous windows if the indirection shows up in profiles.
//!
//! For a `Call`, the VM reads the operand registers from the list and writes
//! them into the callee frame's parameter registers `0..argc`; the result lands
//! in `dst`. Tail calls reuse the frame, marshalling args into the param window
//! in place.

#![allow(dead_code)]

use crate::program::GlobalSlot;

/// A register index within a frame's register file.
pub type Reg = u16;

// Index types for the various pools, all `u32` (or `u16` where the count is
// small). Previously lived in the now-removed stack `instruction` module.
pub type ConstIdx = u32;
pub type BytesIdx = u32;
pub type GlobalIdx = u32;
pub type FuncIdx = u32;
pub type Offset = u32;
/// Index into `Program::field_lists`.
pub type FieldListIdx = u32;

/// Index into `Program::reg_lists` — a list of operand (or destination)
/// registers for a multi-operand instruction.
pub type RegListIdx = u32;

/// The machine representation of a register's value. Mirrors `ir::Repr`; the VM
/// keeps its own copy so it needn't depend on the `ir` crate (codegen
/// translates `ir::Repr -> RegRepr`). `Boxed` registers live in the frame's
/// `Value` window, the rest in the raw `u64` window (bit-reinterpreted).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RegRepr {
	#[default]
	Boxed,
	I64,
	F64,
	I32,
}

/// The register-VM instruction set. Three-address: every operand is an explicit
/// register, every result names its `dst`. Mirrors the 71 stack opcodes
/// (`crate::instruction::Instruction`), minus the stack-only `Pop`/`Dup`/
/// `LoadLocal`/`StoreLocal` (subsumed by direct register addressing + `Move`).
/// Stays `Copy`: variable-length operand lists live in `Program::reg_lists`
/// (via `RegListIdx`) and record field names in `Program::field_lists`.
#[derive(Clone, Copy, Debug)]
pub enum Instruction {
	// --- register moves ---------------------------------------------------
	/// `dst = src`. Register-to-register copy — join points, binding a matched
	/// subject, materializing a captured/global value into its var register.
	Move {
		dst: Reg,
		src: Reg,
	},
	/// `dst = box(src)` — raw i64 window -> boxed `Value::Int`. The repr coercion
	/// pass (M5) inserts these where an unboxed int flows into a boxed context.
	Box {
		dst: Reg,
		src: Reg,
	},
	/// `dst = unbox(src)` — boxed `Value::Int` -> raw i64 window. M5.
	Unbox {
		dst: Reg,
		src: Reg,
	},
	/// Raw-window register copy (both `dst` and `src` are i64-raw). M5.
	MoveR {
		dst: Reg,
		src: Reg,
	},

	// --- constants and immediates ----------------------------------------
	LoadConst {
		dst: Reg,
		k: ConstIdx,
	},
	LoadBytes {
		dst: Reg,
		k: BytesIdx,
	},
	LoadInt {
		dst: Reg,
		val: i64,
	},
	/// Load an int constant into the raw i64 window. M5 (the int-const repr is I64).
	LoadIntR {
		dst: Reg,
		val: i64,
	},
	LoadFloat {
		dst: Reg,
		val: f64,
	},
	LoadBool {
		dst: Reg,
		val: bool,
	},
	/// A duration constant, in nanoseconds.
	LoadDuration {
		dst: Reg,
		ns: i64,
	},
	LoadNothing {
		dst: Reg,
	},

	// --- non-register reads ----------------------------------------------
	/// `dst = captures[idx]`. Emitted once per capture in the function prologue
	/// to materialize each capture into its register; the body then reads it as
	/// an ordinary register.
	LoadCapture {
		dst: Reg,
		idx: u16,
	},
	/// `dst = globals[idx]` (forcing the lazy thunk on first access).
	LoadGlobal {
		dst: Reg,
		idx: GlobalIdx,
	},

	// --- control flow (absolute offsets into the current function) --------
	Jump {
		target: Offset,
	},
	/// If `cond` is false, jump to `target`; else fall through.
	JumpIfFalse {
		cond: Reg,
		target: Offset,
	},

	// --- closures ---------------------------------------------------------
	/// `dst = closure(fn_idx, captures = reg_lists[captures])`.
	MakeClosure {
		dst: Reg,
		fn_idx: FuncIdx,
		captures: RegListIdx,
	},
	/// Like `MakeClosure` but builds a `Value::AsyncFn` (an async-bearing fn
	/// whose `fn_idx` is its resumable step/poll fn). Calling it yields a cold
	/// `Task` rather than running.
	MakeAsyncClosure {
		dst: Reg,
		fn_idx: FuncIdx,
		captures: RegListIdx,
	},

	// --- calls (see the operand-passing note above) -----------------------
	/// `dst = callee(args = reg_lists[args])`, callee in a register.
	Call {
		dst: Reg,
		callee: Reg,
		args: RegListIdx,
	},
	/// Statically-resolved call (`resolve_direct_calls`, on from M4): no callee
	/// register, no closure allocation — the win that made `resolve` worth
	/// turning back on for the register VM.
	CallDirect {
		dst: Reg,
		fn_idx: FuncIdx,
		args: RegListIdx,
	},
	/// Tail call through a callee register. For a closure callee the frame is
	/// reused and this returns directly (the following `Return` is dead); for a
	/// builtin/ctor/async-fn callee (no frame to reuse) the produced value is
	/// written to `dst`, which the following `Return` reads — so `dst` is the
	/// result register the IR's `Let(dst, TailCall); Return(dst)` pair names.
	TailCall {
		dst: Reg,
		callee: Reg,
		args: RegListIdx,
	},
	/// Statically-resolved tail call (M4).
	TailCallDirect {
		dst: Reg,
		fn_idx: FuncIdx,
		args: RegListIdx,
	},
	/// Return `src` to the caller (after running this frame's deferred cleanups).
	/// `raw` is set when the function's return repr is unboxed i64 (M6) — then
	/// `src` is read from the raw window and delivered to the caller's `dst` there.
	Return {
		src: Reg,
		raw: bool,
	},
	/// Push the zero-arg closure in `thunk` onto the frame's `defer` cleanup
	/// stack (run LIFO at `Return`).
	PushDefer {
		thunk: Reg,
	},
	/// Suspension point for the Await-snapshot async path. Kept through M1–M2
	/// (all-boxed, so snapshotting the boxed register window is sound); deleted
	/// at M3 when `cps_transform` runs on the VM path. `task` holds the awaited
	/// `Task`; on resume the awaited result lands in `dst`.
	Await {
		dst: Reg,
		task: Reg,
	},

	// --- aggregates -------------------------------------------------------
	MakeTuple {
		dst: Reg,
		items: RegListIdx,
	},
	MakeList {
		dst: Reg,
		items: RegListIdx,
	},
	/// Concatenate the lists in `reg_lists[lists]` (in order) into one. For
	/// `[...spread]` literals.
	ConcatLists {
		dst: Reg,
		lists: RegListIdx,
	},
	/// Field values in `reg_lists[values]`, names in `field_lists[fields]`
	/// (parallel).
	MakeRecord {
		dst: Reg,
		values: RegListIdx,
		fields: FieldListIdx,
	},
	/// `{ ...record, f: v }`: copy `record`, overwrite the `fields`-named slots
	/// with the override values in `reg_lists[values]`.
	UpdateRecord {
		dst: Reg,
		record: Reg,
		values: RegListIdx,
		fields: FieldListIdx,
	},
	MakeVariant {
		dst: Reg,
		qualified: ConstIdx,
		variant: ConstIdx,
		payload: RegListIdx,
	},
	/// Partial-application constructor for `enum.variant` with payload; building
	/// the variant happens when it's `Call`ed.
	MakeVariantCtor {
		dst: Reg,
		qualified: ConstIdx,
		variant: ConstIdx,
		arity: u16,
	},

	// --- field / element / dict access -----------------------------------
	GetField {
		dst: Reg,
		record: Reg,
		name: ConstIdx,
	},
	GetElement {
		dst: Reg,
		tuple: Reg,
		index: u16,
	},
	/// Read method `index` from a `Value::MethodDict` (trait declaration order).
	GetDictField {
		dst: Reg,
		dict: Reg,
		index: u16,
	},
	/// Build a `Value::MethodDict` from the methods in `reg_lists[methods]`.
	MakeDict {
		dst: Reg,
		methods: RegListIdx,
	},

	/// String interpolation: Display-join the values in `reg_lists[parts]`.
	Interpolate {
		dst: Reg,
		parts: RegListIdx,
	},

	// --- pattern dispatch -------------------------------------------------
	// Each tests `subject`; on failure jumps to `on_fail`. Destructuring forms
	// extract the matched payload directly into the destination registers in
	// `reg_lists[dests]` (one per field) — for a `Bind` sub-pattern that's the
	// bound var's register, otherwise a fresh temp the codegen then recurses on.
	// On the fail path nothing is written, so there is no stack to unwind: the
	// register VM drops the stack VM's reverse-order matching + cleanup
	// trampolines entirely.
	MatchInt {
		subject: Reg,
		val: i64,
		on_fail: Offset,
	},
	MatchFloat {
		subject: Reg,
		val: f64,
		on_fail: Offset,
	},
	MatchDuration {
		subject: Reg,
		ns: i64,
		on_fail: Offset,
	},
	MatchString {
		subject: Reg,
		k: ConstIdx,
		on_fail: Offset,
	},
	MatchBytes {
		subject: Reg,
		k: BytesIdx,
		on_fail: Offset,
	},
	MatchBool {
		subject: Reg,
		val: bool,
		on_fail: Offset,
	},
	MatchNothing {
		subject: Reg,
		on_fail: Offset,
	},
	/// On match, payload field `i` -> `reg_lists[dests][i]`.
	MatchVariant {
		subject: Reg,
		variant: ConstIdx,
		dests: RegListIdx,
		on_fail: Offset,
	},
	/// On match (tuple of arity `dests.len()`), element `i` -> `dests[i]`.
	MatchTuple {
		subject: Reg,
		dests: RegListIdx,
		on_fail: Offset,
	},
	/// List match. Leading elements -> the first `dests`; if `has_rest`, the
	/// remainder list -> the last entry of `dests`. Length-checked.
	MatchList {
		subject: Reg,
		dests: RegListIdx,
		has_rest: bool,
		on_fail: Offset,
	},
	/// Named field values (`field_lists[fields]`) -> the first `dests`; if
	/// `with_rest`, a fresh record of the remaining fields -> the last `dests`.
	/// `exact` rejects extra fields. `exact`/`with_rest` are mutually exclusive.
	MatchRecord {
		subject: Reg,
		fields: FieldListIdx,
		dests: RegListIdx,
		exact: bool,
		with_rest: bool,
		on_fail: Offset,
	},

	// --- arithmetic (split int/float by the analyzer's resolution) --------
	AddInt {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	AddFloat {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	SubInt {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	SubFloat {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	MulInt {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	MulFloat {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	DivInt {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	DivFloat {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	RemInt {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	RemFloat {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	NegInt {
		dst: Reg,
		a: Reg,
	},
	NegFloat {
		dst: Reg,
		a: Reg,
	},

	/// String concatenation (`++`).
	ConcatString {
		dst: Reg,
		a: Reg,
		b: Reg,
	},

	// --- comparisons (ordering split by operand repr; Eq/Neq structural) --
	LtInt {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	LtFloat {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	LteInt {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	LteFloat {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	GtInt {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	GtFloat {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	GteInt {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	GteFloat {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	Eq {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	Neq {
		dst: Reg,
		a: Reg,
		b: Reg,
	},

	// --- logical ----------------------------------------------------------
	LogicalAnd {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	LogicalOr {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	LogicalNot {
		dst: Reg,
		a: Reg,
	},

	// --- M5: unboxed i64 arithmetic/comparison ----------------------------
	// Operands and (for arithmetic) `dst` are in the raw i64 window — no enum
	// tag, no `Value` move, no allocation. Emitted in coerced functions where the
	// repr pass proved the operands unbox to I64. Comparisons read raw i64 and
	// write a boxed `Value::Bool` to `dst` (bools stay boxed in this scope).
	AddIntR {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	SubIntR {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	MulIntR {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	DivIntR {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	RemIntR {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	NegIntR {
		dst: Reg,
		a: Reg,
	},
	LtIntR {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	LteIntR {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	GtIntR {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
	GteIntR {
		dst: Reg,
		a: Reg,
		b: Reg,
	},
}

/// A compiled register-VM program. Mirrors `crate::program::Program`; the
/// function bodies change shape (register instructions + register descriptors)
/// and a `reg_lists` operand pool is added. All other auxiliary tables
/// (constants, globals, field lists, enum/test metadata, the CPS rollout) are
/// identical and reused verbatim.
#[derive(Clone)]
pub struct Program {
	pub functions: Vec<Function>,
	pub constants: Vec<std::rc::Rc<String>>,
	pub bytes_constants: Vec<std::rc::Rc<Vec<u8>>>,
	pub globals: Vec<GlobalSlot>,
	pub field_lists: Vec<Vec<u32>>,
	/// Operand/destination register lists for multi-operand instructions,
	/// indexed by `RegListIdx`.
	pub reg_lists: Vec<Vec<Reg>>,
	pub global_by_name: std::collections::HashMap<(String, String), u32>,
	pub enum_variants: std::collections::HashMap<String, Vec<(String, usize)>>,
	pub entry: u32,
	pub test_suites: Vec<(String, u32)>,
	pub test_new: Option<u32>,
	pub async_poll: Vec<Option<u32>>,
	/// Whether *any* function has an unboxed (`I64`) register — i.e. the repr
	/// coercion pass (M5/M6) is active. When `false` (the shipping default, since
	/// unboxing is a net loss for the VM — see notes/REGISTER_VM.md), the VM never
	/// touches its parallel raw window: no per-call resize, no reads. The raw
	/// machinery stays a zero-cost dormant capability behind this flag.
	pub uses_raw: bool,
}

#[derive(Clone)]
pub struct Function {
	pub name: String,
	pub module: String,
	pub param_count: u16,
	/// Size of the register file (the frame window). Replaces `slot_count`;
	/// the linear-scan allocator (M2) shrinks this below the naive
	/// one-per-`VarId` count.
	pub nregs: u16,
	pub capture_count: u16,
	/// Per-register machine representation, indexed by `Reg` (length `nregs`).
	/// All `Boxed` until the repr coercion pass is turned on for the VM (M5).
	pub reg_reprs: Vec<RegRepr>,
	pub body: Vec<Instruction>,
	/// Per-instruction source ranges for diagnostics. Same length as `body`.
	pub source_ranges: Vec<compiler::Range>,
}
