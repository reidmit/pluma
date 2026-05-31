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

#[derive(Clone)]
pub struct Program {
	pub functions: Vec<Function>,
	pub constants: Vec<Rc<String>>,
	// Bytes-literal constants, indexed by BytesIdx (separate from `constants`
	// because bytes have no UTF-8 invariant and never share storage with
	// strings).
	pub bytes_constants: Vec<Rc<Vec<u8>>>,
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
	// Test suites found in the entry modules: `(module_name, global_idx)`
	// where the global holds that module's `tests` value — a `core.testing`
	// suite, i.e. a `fun registrar -> nothing`. `pluma test` calls each with
	// a fresh registrar, drains the registered cases, and runs them. Module
	// name lets the runner group results.
	pub test_suites: Vec<(String, u32)>,
	// Global index of `core.testing.new` (the registrar builder), set when
	// that module is loaded. The runner calls it once per suite to get a
	// fresh registrar to thread in.
	pub test_new: Option<u32>,
	// The async CPS rollout table (`ir::cps`), indexed by function index.
	// `Some(poll_fn)` means function `i` was rewritten to poll style: the task
	// driver advances it by calling `poll_fn` (`drive_poll`) rather than
	// snapshotting its frame (`drive_step`). `None` (the default, and every
	// entry when the CPS pass didn't run) keeps the Await-style driver. May be
	// shorter than `functions` (treated as `None` past its end).
	pub async_poll: Vec<Option<u32>>,
}

#[derive(Clone)]
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

#[derive(Clone)]
pub enum GlobalSlot {
	// First access runs the function at fn_idx (a zero-arity, no-captures
	// thunk that returns the def's value).
	Pending(u32),
	Evaluating,
	Evaluated(crate::value::Value),
}
