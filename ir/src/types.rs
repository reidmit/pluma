// IR data types. See the crate-level docs in `lib.rs` for the design.
//
// The exact node set is expected to evolve during the lowering port
// (phase 1.1) and again when the WASM backend lands (step 2). The shapes here
// match the sketch in `IR.md`; anything marked "provisional" is known to need
// refinement once real lowering exercises it.

use compiler::Range;
use std::collections::HashMap;

// --------------------------------------------------------------------------
// Identifiers. Abstract handles assigned by lowering; each backend maps them
// to its own storage (the bytecode emitter assigns `VarId`s to VM stack
// slots, a WASM emitter would map them to locals).
// --------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FuncId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlobalId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VarId(pub u32);

/// Identifies a scheduled `defer` cleanup within a function, so the same
/// cleanup can be emitted on every exit edge (normal return + `try`-failure
/// short-circuit) — mirroring the VM's per-frame LIFO cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeferId(pub u32);

// --------------------------------------------------------------------------
// Program.
// --------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct IrProgram {
	/// All functions: top-level defs' thunks, lifted closures, alias
	/// constructors, and the entry function.
	pub functions: Vec<Function>,
	/// One per global slot, in slot order; indexed by `GlobalId`.
	pub globals: Vec<GlobalInit>,
	/// Module-level enum definitions: qualified-enum-name ->
	/// [(variant_name, payload_arity), ...]. Target-independent; used by
	/// pattern compilation and variant construction.
	pub enums: HashMap<String, Vec<(String, usize)>>,
	/// The program entry point.
	pub entry: FuncId,
	/// `core.testing` suites discovered in entry modules: (module, global).
	pub test_suites: Vec<(String, GlobalId)>,
	/// `core.testing.new`'s global, when that module was compiled — the
	/// registrar the test runner threads into each suite. `None` for programs
	/// that don't pull in `core.testing`.
	pub test_new: Option<GlobalId>,
}

/// How a global slot is initialized.
///
/// Provisional: the value-encoding for pre-evaluated globals (prelude trait
/// dictionaries, native-module values/constants) is finalized during the
/// lowering port. For now this captures the structural cases the emitter must
/// distinguish.
#[derive(Debug, Clone)]
pub enum GlobalInit {
	/// Computed at load time by running a thunk function (top-level defs).
	Thunk(FuncId),
	/// A value the backend constructs directly, no thunk needed.
	PreEvaluated(PreEval),
}

/// A pre-evaluated global value. Provisional (see `GlobalInit`).
#[derive(Debug, Clone)]
pub enum PreEval {
	/// A primitive dispatched by tag (`print`, `int-add`, native-module defs).
	Builtin(String),
	/// A primitive constant (native-module constants like `math.pi`).
	Const(Const),
	/// A trait instance method dictionary: positional method values in trait
	/// declaration order (e.g. `numeric` is `add, sub, mul, div, negate`).
	MethodDict(Vec<PreEval>),
}

// --------------------------------------------------------------------------
// Functions.
// --------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Function {
	pub name: String,
	pub module: String,
	/// Formal parameters, in order. Trait-constrained functions carry their
	/// dictionary parameters here too (dictionary passing is made explicit by
	/// lowering).
	pub params: Vec<VarId>,
	/// Free variables captured from the enclosing scope, in order. Produced by
	/// closure conversion; each backend realizes the environment its own way.
	pub captures: Vec<VarId>,
	/// True if the function awaits (its body contains `Await`). Drives
	/// `MakeAsyncClosure` in the bytecode emitter; the seam for the step-2 CPS
	/// state-machine pass.
	pub is_async: bool,
	pub body: Block,
	/// Representation of every `VarId` defined in this function, indexed by
	/// `VarId.0` (params, captures, and every `Let`/pattern-bound var). Produced
	/// by the step-2 Repr inference pass (`repr::infer_reprs`). Under the
	/// uniform-boxed-first scheme every binding is `Boxed` except the results of
	/// arithmetic/comparison/`Not` ops and primitive `Const` literals. The
	/// bytecode emitter ignores this (the VM is uniformly boxed); the WASM backend
	/// maps each repr to an i64/f64/i32 or GC-ref local. Empty until inference runs.
	pub var_reprs: Vec<Repr>,
	/// The representation of each formal parameter, parallel to `params`. The
	/// projection of each param's resolved type (`repr::repr_of_type`), recorded by
	/// lowering. All-`Boxed` (the uniform-boxed contract) until the step-2
	/// monomorphization pass stamps eligible, non-escaping concrete functions with
	/// their unboxed signature — at which point an `int` param reads as `I64`,
	/// killing the entry box/unbox churn. The bytecode emitter ignores it.
	pub param_reprs: Vec<Repr>,
	/// The representation of the function's return value — the projection of the
	/// body's tail type. `Boxed` until monomorphization stamps it. Drives the
	/// `Return`-site repr requirement in the coercion pass; ignored by the VM.
	pub ret_repr: Repr,
}

/// The machine representation a value takes. The bytecode VM is uniformly
/// `Boxed` (its `Value` enum is already inline-tagged, so the distinction is
/// invisible there); a WASM backend maps `I64`/`F64`/`I32` to native locals and
/// `Boxed` to a GC reference. Assigned per `VarId` by `repr::infer_reprs` and
/// bridged by the `Box`/`Unbox` rvalues the coercion pass inserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repr {
	/// A heap reference / uniform `Value` — every polymorphic or compound value.
	Boxed,
	/// An unboxed `int`.
	I64,
	/// An unboxed `float`.
	F64,
	/// An unboxed `bool`.
	I32,
}

/// A straight-line sequence of statements. Control flow lives in `Stmt`
/// (structured), not in block edges.
#[derive(Debug, Clone)]
pub struct Block(pub Vec<Stmt>);

/// One IR statement, plus the source range of the AST expression that produced
/// it. Backends pin every emitted instruction to this range so the VM can
/// attribute runtime errors and `debug` call sites to the right line. The range
/// is `Range::collapsed(0, 0)` for synthetic stmts (entry function, poison
/// thunk, dict-builder/ctor scaffolding) that have no source origin.
#[derive(Clone)]
pub struct Stmt {
	pub kind: StmtKind,
	pub range: Range,
}

// `Range`'s own `Debug` is gated on `debug_assertions`, so deriving `Debug`
// here would break release builds (and everything that transitively derives it:
// `Block`, `Function`, `IrProgram`). Format the range from its plain `usize`
// fields instead, keeping the whole IR `Debug`-printable in every profile.
impl std::fmt::Debug for Stmt {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Stmt")
			.field("kind", &self.kind)
			.field(
				"range",
				&format_args!(
					"{}:{}..{}:{}",
					self.range.start.line, self.range.start.col, self.range.end.line, self.range.end.col
				),
			)
			.finish()
	}
}

impl Stmt {
	pub fn new(kind: StmtKind, range: Range) -> Self {
		Self { kind, range }
	}

	/// A stmt with no source-level origin — entry/poison/dict scaffolding.
	pub fn synthetic(kind: StmtKind) -> Self {
		Self {
			kind,
			range: Range::collapsed(0, 0),
		}
	}
}

#[derive(Debug, Clone)]
pub enum StmtKind {
	/// Bind the result of an `Rvalue` to a fresh variable (ANF).
	Let(VarId, Rvalue),
	/// Two-way branch on a boolean atom.
	If(Atom, Block, Block),
	/// Multi-way branch on an integer discriminant (enum tag / literal),
	/// produced by pattern-match compilation.
	Switch {
		scrutinee: Atom,
		arms: Vec<(i64, Block)>,
		default: Box<Block>,
	},
	/// Pattern match on `subject`: the first arm whose pattern matches runs.
	/// Arms are tried in order; if none match, control falls through (the IR
	/// for `when`/`if` arranges a result default of `nothing`, matching the VM).
	/// Kept at the pattern level (rather than pre-compiled to `Switch`) so each
	/// backend chooses its own compilation; a decision-tree IR->IR pass can
	/// rewrite it later.
	Match {
		subject: Atom,
		arms: Vec<MatchArm>,
	},
	/// Structured loop; exits via `Break`, iterates via `Continue`.
	Loop(Block),
	Break,
	Continue,
	/// Return an atom from the enclosing function.
	Return(Atom),
	/// Evaluate an `Rvalue` for its effect, discarding the result.
	Discard(Rvalue),
	/// Run a previously-scheduled `defer` cleanup. Emitted on each exit edge.
	RunDefer(DeferId),
	/// Schedule a zero-arg cleanup closure on the running frame's cleanup
	/// stack — `defer expr` lowers to a closure of `fun { expr }` plus a
	/// `PushDefer`. The VM walks the cleanup stack LIFO at `Return` (and on
	/// `try`-failure short-circuit).
	PushDefer(Atom),
}

/// One arm of a `Match`: a pattern and the block to run on a match.
#[derive(Debug, Clone)]
pub struct MatchArm {
	pub pattern: Pattern,
	pub body: Block,
}

/// A match pattern. Sub-patterns are full patterns (so nesting works); the
/// only unsupported kind is string-interpolation patterns.
#[derive(Debug, Clone)]
pub enum Pattern {
	/// Matches anything, binds nothing (`_`, or the `else` arm).
	Wildcard,
	/// Matches anything, binds the subject to a variable.
	Bind(VarId),
	/// Matches a constant by value.
	Literal(Const),
	/// Matches an enum variant by name; each payload field is a sub-pattern.
	Variant {
		variant: String,
		fields: Vec<Pattern>,
	},
	/// Matches a tuple of the given arity; elements are sub-patterns.
	Tuple(Vec<Pattern>),
	/// Matches a list. `items` are the leading element patterns; `rest`
	/// captures the remainder (`None` = exact length).
	List {
		items: Vec<Pattern>,
		rest: Option<ListRest>,
	},
	/// Matches a record carrying (at least) the named fields.
	Record {
		fields: Vec<(String, Pattern)>,
		rest: RecordRest,
	},
}

/// The `...` tail of a list pattern.
#[derive(Debug, Clone)]
pub enum ListRest {
	/// `...` — matches any remainder, binds nothing.
	Anon,
	/// `...name` — binds the remainder as a list.
	Bind(VarId),
}

/// The tail behavior of a record pattern.
#[derive(Debug, Clone)]
pub enum RecordRest {
	/// `{a, b}` — the record must have exactly these fields.
	Exact,
	/// `{a, ...}` — extra fields allowed, not captured.
	Open,
	/// `{a, ...rest}` — extra fields captured into `rest`.
	Bind(VarId),
}

/// A trivially-evaluable operand: a variable or an inline constant. Atoms have
/// no side effects and never allocate, so they can appear freely as call
/// arguments and operands.
#[derive(Debug, Clone)]
pub enum Atom {
	Var(VarId),
	Const(Const),
}

/// An inline constant value.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
	Unit,
	Bool(bool),
	Int(i64),
	Float(f64),
	Str(String),
	Bytes(Vec<u8>),
	/// A duration literal, in nanoseconds (the underlying `Value::Duration` rep).
	Duration(i64),
}

/// An operation that may compute, call, or allocate. Always `Let`- or
/// `Discard`-bound (ANF), so its evaluation point is explicit.
#[derive(Debug, Clone)]
pub enum Rvalue {
	/// The value of an atom (a move/copy).
	Use(Atom),
	/// A strict binary operation. The operand types are already resolved by
	/// the analyzer, so e.g. integer vs float addition is distinct here.
	Bin(BinOp, Atom, Atom),
	/// Logical negation (`!`).
	Not(Atom),
	/// Call a statically-known target.
	Call(Callee, Vec<Atom>),
	/// Call through a closure value.
	CallClosure(Atom, Vec<Atom>),
	/// A tail call through a closure value: the call is in tail position (its
	/// result is the enclosing function's return value). Lowers to the VM's
	/// `TailCall`, which reuses the current frame for a closure callee (so the
	/// following `Return` is dead) and falls back to a plain call for
	/// builtins/ctors/async-fns. Produced only by `lower`'s tail path; always
	/// immediately followed by a `Return` of its result.
	TailCall(Atom, Vec<Atom>),
	/// Read method `index` (trait declaration order) from a dictionary value,
	/// yielding a callable.
	GetDictMethod(Atom, u32),
	/// Build a trait-instance method dictionary from its method values, in
	/// trait declaration order. Produced when lowering an `instance` def.
	MakeDict(Vec<Atom>),
	/// Allocate a closure: a code pointer plus captured values, in
	/// `Function::captures` order.
	MakeClosure(FuncId, Vec<Atom>),
	/// Build a record from (field-name, value) pairs. Provisional: record-slot
	/// lowering (a later pass) replaces field names with static slot indices.
	MakeRecord(Vec<(String, Atom)>),
	GetField(Atom, String),
	/// Construct an enum variant with all its payload present.
	MakeVariant {
		enum_name: String,
		tag: u32,
		payload: Vec<Atom>,
	},
	/// A variant *constructor* value (for a variant with payload referenced
	/// without all its arguments, e.g. bare `some` or `ok`). Calling it builds
	/// the variant.
	MakeVariantCtor {
		enum_name: String,
		tag: u32,
	},
	/// String interpolation: combine the parts (already `to-string`'d where
	/// needed by the analyzer) into one string.
	Interpolate(Vec<Atom>),
	/// A regex literal, carried as its compiled pattern string (the
	/// target-independent form — each backend feeds it to its own engine).
	Regex(String),
	/// Read a variant's discriminant tag (for `Switch`).
	GetTag(Atom),
	/// Read field `index` of a variant's payload.
	GetPayload(Atom, u32),
	MakeList(Vec<ListItem>),
	MakeTuple(Vec<Atom>),
	/// Load a global (top-level def, prelude dict, native value).
	GlobalRef(GlobalId),
	/// A primitive dispatched by tag, as a first-class value.
	Builtin(String),
	/// Await a task. Explicit in step 1 (the bytecode emitter handles it via
	/// the existing async-closure path); rewritten into a state machine by the
	/// step-2 CPS pass.
	Await(Atom),
	/// Box an unboxed value into a uniform heap `Value`. A no-op on the bytecode
	/// VM (its `Value` is already tagged); inserted by the Repr coercion pass
	/// where an unboxed value flows into a `Boxed` context (a `Call` argument, a
	/// container element, a `Return`, …). The WASM backend emits the actual
	/// i64/f64/i32 → GC-ref boxing.
	Box(Atom),
	/// Unbox a heap `Value` to the named primitive repr. A no-op on the bytecode
	/// VM; inserted where a `Boxed` value feeds a repr-typed op (e.g. an `AddInt`
	/// operand). The `Repr` is always `I64`/`F64`/`I32`, never `Boxed`.
	Unbox(Atom, Repr),
}

/// A statically-resolved call target.
#[derive(Debug, Clone)]
pub enum Callee {
	Function(FuncId),
	Global(GlobalId),
	Builtin(String),
}

/// An element of a list literal. Mirrors the AST's `...` spread support.
#[derive(Debug, Clone)]
pub enum ListItem {
	Elem(Atom),
	Spread(Atom),
}

/// Strict binary operators, mirroring the VM's binary opcodes. Arithmetic is
/// split by operand type (the analyzer picks int vs float); comparison and
/// equality are single ops (the VM implements them polymorphically over the
/// `ord`/structural semantics). Logical `and`/`or` are strict here (both
/// operands are evaluated) — that matches the VM's `LogicalAnd`/`LogicalOr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
	AddInt,
	SubInt,
	MulInt,
	DivInt,
	RemInt,
	AddFloat,
	SubFloat,
	MulFloat,
	DivFloat,
	RemFloat,
	/// String concatenation (`++`).
	Concat,
	And,
	Or,
	/// Structural equality / inequality — operands are `Boxed` (works on any
	/// value: ints, strings, records, …; the VM compares `Value`s structurally).
	Eq,
	Ne,
	// Ordering comparisons, split by operand repr so the representation is
	// explicit in the op (the VM has one polymorphic opcode per relation, but
	// WASM needs `i64.lt`/`f64.lt`, and the coercion pass needs to know whether a
	// boxed operand unboxes to I64 or F64). All map back to the VM's
	// `Lt`/`Lte`/`Gt`/`Gte` opcodes. `*I64` operands are `I64`, `*F64` are `F64`;
	// the result is always `I32` (bool).
	LtI64,
	LtF64,
	LeI64,
	LeF64,
	GtI64,
	GtF64,
	GeI64,
	GeF64,
}

#[cfg(test)]
mod tests {
	use super::*;

	// Exercises the type set end-to-end: builds the IR for a function shaped
	// like `fun x { x + 1 }` and confirms the nodes compose as intended.
	#[test]
	fn builds_a_trivial_function() {
		let arg = VarId(0);
		let sum = VarId(1);
		let body = Block(vec![
			Stmt::synthetic(StmtKind::Let(
				sum,
				Rvalue::Bin(BinOp::AddInt, Atom::Var(arg), Atom::Const(Const::Int(1))),
			)),
			Stmt::synthetic(StmtKind::Return(Atom::Var(sum))),
		]);
		let f = Function {
			name: "inc".into(),
			module: "main".into(),
			params: vec![arg],
			captures: vec![],
			is_async: false,
			body,
			var_reprs: vec![],
			param_reprs: vec![],
			ret_repr: Repr::Boxed,
		};

		assert_eq!(f.params, vec![VarId(0)]);
		assert!(!f.is_async);
		match &f.body.0[0].kind {
			StmtKind::Let(v, Rvalue::Bin(op, _, _)) => {
				assert_eq!(*v, VarId(1));
				assert_eq!(*op, BinOp::AddInt);
			}
			other => panic!("expected Let(Bin), got {other:?}"),
		}
	}
}
