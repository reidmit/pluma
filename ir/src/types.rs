// IR data types. See the crate-level docs in `lib.rs` for the design.
//
// Node shapes marked "provisional" are known to need refinement as more of
// the backend exercises them.

use compiler::Range;
use std::collections::HashMap;

// --------------------------------------------------------------------------
// Identifiers. Abstract handles assigned by lowering; the WASM emitter maps
// them to locals.
// --------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FuncId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlobalId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VarId(pub u32);

/// Identifies a scheduled `defer` cleanup within a function, so the same
/// cleanup can be emitted on every exit edge (normal return + `try`-failure
/// short-circuit), in LIFO order.
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
	/// `tests` suites (`std.test`) discovered in entry modules: (module, global).
	/// `lower_tests` synthesizes the entry over these; `pluma test` also reads it to
	/// detect "no tests found".
	pub test_suites: Vec<(String, GlobalId)>,
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
	/// A primitive dispatched by tag (`print`, `int-add`, native-module defs),
	/// carrying the `Repr` of its *declared* return type (computed from the
	/// `built-in` def's type annotation at lower time). Threading the repr from
	/// the analyzer here is what lets a builtin call be resolved to a typed
	/// `Callee::Builtin` without rediscovering the type — see `resolve_builtins`.
	/// Polymorphic-returning builtins (`list.get : (list a) int -> a`) carry
	/// `Boxed` (the declared `a`), which is correct: the value comes boxed out of
	/// the container regardless of the concrete type at the call site.
	Builtin(String, Repr),
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
	/// `MakeAsyncClosure` emission and is the input the CPS state-machine pass
	/// keys on.
	pub is_async: bool,
	/// The CPS state-machine rollout marker (`ir::cps`). `None` in freshly-lowered
	/// IR. `wasm::emit`'s async-lowering pass (`ir::cps::cps_transform`, driven by
	/// `wasm/src/async_lower.rs`) rewrites every awaiting function to poll style and
	/// sets this to `Some(poll_fn)`: the function stays in place (still drives
	/// `MakeAsyncClosure`/`do_call`, so callers are unchanged), but the driver
	/// advances it by calling `poll_fn` — `poll(state, resume) -> __poll`. The
	/// referenced `poll_fn` is an ordinary (`poll_fn: None`) 2-arg function. Set by
	/// the CPS pass; `None` until it runs.
	pub poll_fn: Option<FuncId>,
	pub body: Block,
	/// Representation of every `VarId` defined in this function, indexed by
	/// `VarId.0` (params, captures, and every `Let`/pattern-bound var). Produced
	/// by the Repr inference pass (`repr::infer_reprs`, run by `wasm::emit`). Under
	/// the uniform-boxed-first scheme every binding is `Boxed` except the results of
	/// arithmetic/comparison/`Not` ops and primitive `Const` literals. The WASM
	/// backend maps each repr to an i64/f64/i32 or GC-ref local. Empty until
	/// inference runs.
	pub var_reprs: Vec<Repr>,
	/// The representation of each formal parameter, parallel to `params`. The
	/// projection of each param's resolved type (`repr::repr_of_type`), recorded by
	/// lowering. All-`Boxed` (the uniform-boxed contract): the interprocedural
	/// unboxing pass that once stamped concrete functions with unboxed signatures
	/// was VM-substrate-only and was removed with the VM.
	pub param_reprs: Vec<Repr>,
	/// The representation of the function's return value — the projection of the
	/// body's tail type. `Boxed` (the uniform-boxed contract; see `param_reprs`).
	/// Drives the `Return`-site repr requirement in the coercion pass.
	pub ret_repr: Repr,
}

/// The machine representation a value takes. The WASM backend maps
/// `I64`/`F64`/`I32` to native locals and `Boxed` to a GC reference. Assigned per
/// `VarId` by `repr::infer_reprs` and bridged by the `Box`/`Unbox` rvalues the
/// coercion pass inserts.
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
/// it. The backend pins every emitted instruction to this range so it can
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
	/// for `when`/`if` arranges a result default of `nothing`).
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
	/// `PushDefer`. The cleanup stack is walked LIFO at `Return` (and on
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
		/// The statically-resolved closed shape of the matched record (its
		/// name-sorted field set), when the subject type is a closed record;
		/// `None` for an open (row-polymorphic) subject or any non-closed-record
		/// type. Lets the backend resolve each bound field name to a constant slot
		/// index (`RecordShape::slot_of`) instead of a runtime name-scan. Threaded
		/// by lowering from the subject's resolved type.
		shape: Option<RecordShape>,
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

/// The statically-known shape of a *closed* record at a field-access or pattern
/// site: its field names in canonical name-sorted order — the same order
/// `MakeRecord` lays out its parallel `names`/`values` arrays. Threaded from the
/// analyzer's resolved record type by lowering (`record_shape_of`), and `None`
/// when the receiver/subject type is open (row-polymorphic) or otherwise not a
/// statically-resolved closed record.
///
/// Lets the backend resolve a field name to a constant slot index (`slot_of`) —
/// the basis for the nominal-struct record representation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordShape {
	/// Field names in canonical name-sorted order (matching `MakeRecord`).
	pub fields: Vec<String>,
}

impl RecordShape {
	/// The constant slot index of `field` within this shape, if present.
	pub fn slot_of(&self, field: &str) -> Option<usize> {
		self.fields.iter().position(|f| f == field)
	}
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
	/// result is the enclosing function's return value), so the callee can reuse
	/// the current frame (the following `Return` is dead); falls back to a plain
	/// call for builtins/ctors/async-fns. Produced only by `lower`'s tail path;
	/// always immediately followed by a `Return` of its result.
	TailCall(Atom, Vec<Atom>),
	/// A tail call to a statically-known top-level function — the tail-position
	/// analogue of `Call(Callee::Function(..))`. Produced by `resolve_direct_calls`
	/// from a `TailCall` whose callee is a capture-free non-async global function,
	/// it drops the `LoadGlobal` + indirect dispatch (the callee resolves to a
	/// zero-capture closure, so this is behavior-neutral); always immediately
	/// followed by a `Return` of its result.
	TailCallDirect(FuncId, Vec<Atom>),
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
	/// Record update `{ ...base, f: v }`: copy `base` and override each named
	/// field. The analyzer guarantees every override field already exists on
	/// `base` (update-only, type-preserving), so the result has `base`'s shape.
	RecordUpdate {
		base: Atom,
		fields: Vec<(String, Atom)>,
	},
	/// Read a record field by name. The optional `RecordShape` is the statically-
	/// resolved closed shape of the receiver (its name-sorted field set), threaded
	/// by lowering from the receiver's type; `None` for an open/row-polymorphic
	/// receiver. The backend resolves the field to a constant slot
	/// (`RecordShape::slot_of`).
	GetField(Atom, String, Option<RecordShape>),
	/// Read element `index` of a tuple (`e.0`, `e.1`). The tuple analogue of
	/// `GetField`; index is statically known and bounds-checked by the analyzer
	/// against concrete tuples.
	GetElement(Atom, u32),
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
	/// Await a task. Rewritten into a state machine by the CPS pass (`ir::cps`, run
	/// by `wasm::emit`) before emission — the emitter never sees a raw `Await`.
	Await(Atom),
	/// Box an unboxed value into a uniform heap `Value`. Inserted by the Repr
	/// coercion pass where an unboxed value flows into a `Boxed` context (a `Call`
	/// argument, a container element, a `Return`, …); the WASM backend emits the
	/// actual i64/f64/i32 → GC-ref boxing.
	Box(Atom),
	/// Unbox a heap `Value` to the named primitive repr. Inserted where a `Boxed`
	/// value feeds a repr-typed op (e.g. an `AddInt` operand). The `Repr` is always
	/// `I64`/`F64`/`I32`, never `Boxed`.
	Unbox(Atom, Repr),
}

/// A statically-resolved call target.
#[derive(Debug, Clone)]
pub enum Callee {
	Function(FuncId),
	Global(GlobalId),
	/// A primitive dispatched by tag, carrying its declared return `Repr` (from
	/// the builtin global it was resolved from). The repr lets the coercion pass
	/// read a scalar-returning builtin's result unboxed instead of forcing every
	/// call result `Boxed`. Produced by `resolve_builtins`, part of `wasm::emit`'s
	/// pipeline.
	Builtin(String, Repr),
}

/// An element of a list literal. Mirrors the AST's `...` spread support.
#[derive(Debug, Clone)]
pub enum ListItem {
	Elem(Atom),
	Spread(Atom),
}

/// Strict binary operators. Arithmetic is split by operand type (the analyzer
/// picks int vs float); `Eq`/`Ne` are polymorphic structural ops (over any value),
/// with repr-split numeric variants below. Logical `and`/`or` are strict here
/// (both operands are always evaluated).
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
	/// value: ints, strings, records, …; compared structurally).
	Eq,
	Ne,
	// Concrete int/float equality, split by operand repr so a `==`/`!=` on numbers
	// compares `i64`/`f64` registers directly instead of boxing both sides for the
	// structural `__eq` helper. Behavior-identical to `Eq`/`Ne` on those types: int
	// equality is i64 equality, and concrete float `==`/`!=` is IEEE (`nan != nan`),
	// which is exactly what structural `==`/`!=` gives on floats. WASM emits
	// `i64.eq`/`f64.ne`/… . Result is `I32` (bool).
	EqI64,
	NeI64,
	EqF64,
	NeF64,
	// Ordering comparisons, split by operand repr so the representation is
	// explicit in the op (WASM needs `i64.lt`/`f64.lt`, and the coercion pass needs
	// to know whether a boxed operand unboxes to I64 or F64). `*I64` operands are
	// `I64`, `*F64` are `F64`; the result is always `I32` (bool).
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
			poll_fn: None,
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
