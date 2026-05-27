// IR data types. See the crate-level docs in `lib.rs` for the design.
//
// The exact node set is expected to evolve during the lowering port
// (phase 1.1) and again when the WASM backend lands (step 2). The shapes here
// match the sketch in `IR.md`; anything marked "provisional" is known to need
// refinement once real lowering exercises it.

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
}

/// A straight-line sequence of statements. Control flow lives in `Stmt`
/// (structured), not in block edges.
#[derive(Debug, Clone)]
pub struct Block(pub Vec<Stmt>);

#[derive(Debug, Clone)]
pub enum Stmt {
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
	Eq,
	Ne,
	Lt,
	Le,
	Gt,
	Ge,
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
			Stmt::Let(
				sum,
				Rvalue::Bin(BinOp::AddInt, Atom::Var(arg), Atom::Const(Const::Int(1))),
			),
			Stmt::Return(Atom::Var(sum)),
		]);
		let f = Function {
			name: "inc".into(),
			module: "main".into(),
			params: vec![arg],
			captures: vec![],
			is_async: false,
			body,
		};

		assert_eq!(f.params, vec![VarId(0)]);
		assert!(!f.is_async);
		match &f.body.0[0] {
			Stmt::Let(v, Rvalue::Bin(op, _, _)) => {
				assert_eq!(*v, VarId(1));
				assert_eq!(*op, BinOp::AddInt);
			}
			other => panic!("expected Let(Bin), got {other:?}"),
		}
	}
}
