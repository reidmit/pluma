// A compiled Pluma program.
//
// `functions` indexes every Pluma function (including the synthetic
// "thunks" we generate for each top-level def to support lazy evaluation).
// `entry` is the function index for the top-level call to `main`.
//
// `globals` holds one slot per (module, def_name) pair. Codegen builds a
// stable mapping from (module, def_name) to GlobalIdx and emits
// LoadGlobal(idx) for cross-module and same-module top-level references.
// Each Global starts as Pending(fn_idx) and transitions to Evaluated(v)
// on first access; an Evaluating sentinel catches cycles.
//
// `constants` is the strings/regex-source pool, shared across instructions.

use crate::instruction::Instruction;
use std::rc::Rc;

pub struct Program {
	pub functions: Vec<Function>,
	pub constants: Vec<Rc<String>>,
	// Bytes-literal constants, indexed by BytesIdx (separate from `constants`
	// because bytes have no UTF-8 invariant and never share storage with
	// strings).
	pub bytes_constants: Vec<Rc<Vec<u8>>>,
	pub regex_patterns: Vec<Rc<crate::value::RegexData>>,
	pub globals: Vec<GlobalSlot>,
	// Record-shape field name lists, indexed by FieldListIdx. Moves the
	// only Vec-carrying field out of `Instruction` so the instruction
	// stream stays Copy-sized.
	pub field_lists: Vec<Vec<u32>>,
	// (module_name, def_name) -> GlobalIdx. Used by both codegen (during
	// emission) and the VM (for resolution by name when needed).
	pub global_by_name: std::collections::HashMap<(String, String), u32>,
	// Module-level enum definitions: qualified-enum-name ->
	// [(variant_name, payload_arity), ...]. Used by the VM to disambiguate
	// identifier patterns against the subject's actual variant set.
	pub enum_variants: std::collections::HashMap<String, Vec<(String, usize)>>,
	pub entry: u32,
	// One entry per `test "..." { ... }` block found in the compiled program.
	// `(module_name, description, global_idx)` — the global holds a
	// zero-arity closure the runner invokes once per test. Module name lets
	// `pluma test` group results by test module.
	pub tests: Vec<(String, String, u32)>,
}

pub struct Function {
	pub name: String, // for diagnostics/Display
	// Module the function was emitted from. Used by `debug` to print a
	// `<module>:<line>` header. Empty for the synthetic `__entry__` thunk.
	pub module: String,
	pub param_count: u16,
	pub slot_count: u16, // total locals (params + lets)
	pub capture_count: u16,
	pub body: Vec<Instruction>,
	// Per-instruction source ranges for diagnostics. Same length as `body`.
	pub source_ranges: Vec<compiler::Range>,
}

pub enum GlobalSlot {
	// First access runs the function at fn_idx (a zero-arity, no-captures
	// thunk that returns the def's value).
	Pending(u32),
	Evaluating,
	Evaluated(crate::value::Value),
}
