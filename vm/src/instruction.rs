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

use crate::builtin::Builtin;

pub type SlotIdx = u16;
pub type CaptureIdx = u16;
pub type ConstIdx = u32;
pub type GlobalIdx = u32;
pub type FuncIdx = u32;
pub type Offset = u32;

#[derive(Clone, Debug)]
pub enum Instruction {
	// Stack manipulation
	Pop,
	Dup,

	// Constants and immediates
	LoadConst(ConstIdx),
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
	MakeRecord {
		// Indices into constants pool, each a field name.
		fields: Vec<ConstIdx>,
	},
	MakeVariant {
		qualified: ConstIdx,
		variant: ConstIdx,
		arity: u16,
	},
	GetField(ConstIdx),

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
	// fields; the corresponding values are pushed onto the stack in the
	// order the patterns appear (last on top).
	MatchRecord {
		fields: Vec<ConstIdx>,
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

	// Builtin invocation. The builtin's handler reads `arity` operands off
	// the stack.
	CallBuiltin(Builtin, u16),
}
