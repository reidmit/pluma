// The VM's instruction set. Each function compiles to a Vec<Instruction>;
// the VM dispatches in a tight loop.
//
// Conventions:
// - "Slot" = local-variable index within a frame (params + lets)
// - "Capture" = closure capture index
// - "Global" = top-level def in the program (per-module tables flattened
//   into one)
// - "Const" = entry in the program's constants pool (strings, regex AST,
//   record field names, etc.)
// - Jumps are absolute instruction offsets within the current function.

pub type SlotIdx = u16;
pub type CaptureIdx = u16;
pub type ConstIdx = u32;
pub type BytesIdx = u32;
pub type GlobalIdx = u32;
pub type FuncIdx = u32;
pub type Offset = u32;
// Index into Program::field_lists — used by Make/MatchRecord so the
// instruction itself stays Copy-sized.
pub type FieldListIdx = u32;

#[derive(Clone, Copy, Debug)]
pub enum Instruction {
	// Stack manipulation
	Pop,
	Dup,

	// Constants and immediates
	LoadConst(ConstIdx),
	LoadBytes(BytesIdx),
	LoadInt(i64),
	LoadFloat(f64),
	LoadBool(bool),
	LoadNothing,

	// Variables
	LoadLocal(SlotIdx),
	StoreLocal(SlotIdx),
	LoadCapture(CaptureIdx),
	LoadGlobal(GlobalIdx),

	// Control flow (absolute offsets into the current function)
	Jump(Offset),
	JumpIfFalse(Offset),

	// Functions
	MakeClosure {
		fn_idx: FuncIdx,
		num_captures: u16,
	},
	Call(u16),
	TailCall(u16),
	Return,

	// Aggregates
	MakeTuple(u16),
	MakeList(u16),
	// Field names live in Program::field_lists indexed by the FieldListIdx
	// here. Keeps the instruction Copy.
	MakeRecord(FieldListIdx),
	MakeVariant {
		qualified: ConstIdx,
		variant: ConstIdx,
		arity: u16,
	},
	GetField(ConstIdx),

	// Typeclass dispatch: pop a Value::Dict and push the method at the given
	// field index. Method index is the position the method was declared in
	// its trait body (e.g. for `numeric { add, sub, mul, div, negate }`, add
	// is 0). Compiled after `LoadGlobal(instance_slot)` or `LoadLocal(...)`
	// that puts the dict on top of the stack.
	GetDictField(u16),

	// Build a `Value::Dict` of the given size from the top N stack values.
	// Used when codegen builds an instance dictionary of method closures at
	// first use (concrete user instances) or via an instance-constructor
	// function (parametric instances). Methods are popped in declaration
	// order — the topmost stack value becomes the last dict entry.
	MakeDict(u16),

	// Variant constructor (for partial application of `enum.variant` where
	// the variant has payload). When called via Call, the resulting variant
	// is built.
	MakeVariantCtor {
		qualified: ConstIdx,
		variant: ConstIdx,
		arity: u16,
	},

	// Regex literal: build a Value::Regex from a precompiled pattern.
	LoadRegex(ConstIdx),

	// String interpolation: combine N values on the stack into a single
	// string. Each value gets Display-formatted.
	Interpolate(u16),

	// Pattern dispatch. Each pop the subject; on match-failure, push the
	// subject back so cleanup is uniform — except where we explicitly say
	// otherwise.
	//
	// Convention: subject is on top of stack before the instruction. On
	// MATCH SUCCESS the subject is consumed and (where applicable) its
	// payload is left on the stack in the order needed by sub-patterns.
	// On MATCH FAILURE the subject is consumed and we jump to the offset.
	MatchInt(i64, Offset),
	MatchFloat(f64, Offset),
	MatchString(ConstIdx, Offset),
	MatchBytes(BytesIdx, Offset),
	MatchBool(bool, Offset),
	MatchNothing(Offset),
	// MatchVariant: subject must be a Variant with the given bare variant
	// name; on match its payload values are pushed onto the stack (in
	// order, last on top — so sub-patterns can match them right-to-left
	// with the top corresponding to the last payload arg).
	MatchVariant {
		variant: ConstIdx,
		arity: u16,
		on_fail: Offset,
	},
	// MatchTuple: subject must be a Tuple of the given arity; elements
	// pushed onto the stack (last on top).
	MatchTuple {
		arity: u16,
		on_fail: Offset,
	},
	// MatchRecord: subject must be a Record containing all the named
	// fields. If `exact` is true, the record must have only those fields
	// (no extras). If `with_rest` is true, after pushing the named field
	// values onto the stack a fresh Record containing the input's other
	// fields is pushed on top (used by `{a, ...rest}` patterns). `exact`
	// and `with_rest` are mutually exclusive at the codegen level.
	// Field names live in Program::field_lists.
	MatchRecord {
		fields_idx: FieldListIdx,
		exact: bool,
		with_rest: bool,
		on_fail: Offset,
	},
	// MatchList: subject must be a List. If has_rest is false, the list
	// must have exactly `arity` elements; all are pushed in order (last on
	// top). If has_rest is true, the list must have at least `arity`
	// elements; the first `arity` are pushed in order, then the remainder
	// list is pushed on top as a fresh Value::List.
	MatchList {
		arity: u16,
		has_rest: bool,
		on_fail: Offset,
	},

	// Operators (numeric/comparison/logical).
	// Arithmetic is split int vs float per the analyzer's already-done
	// resolution. Comparisons dispatch on tag at runtime to share opcodes.
	AddInt,
	AddFloat,
	SubInt,
	SubFloat,
	MulInt,
	MulFloat,
	DivInt,
	DivFloat,
	RemInt,
	RemFloat,
	NegInt,
	NegFloat,

	Lt,
	Lte,
	Gt,
	Gte,
	Eq,
	Neq,

	LogicalAnd,
	LogicalOr,
	LogicalNot,
}
