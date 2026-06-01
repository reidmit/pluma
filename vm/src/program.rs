// Shared global-slot state for a compiled program.
//
// The program/function shapes themselves live in `reg` (the register VM). A
// `GlobalSlot` holds one top-level def: it starts `Pending(fn_idx)` (a zero-arity
// thunk that computes the def's value) and transitions to `Evaluated(v)` on first
// access; an `Evaluating` sentinel catches reference cycles.

#[derive(Clone)]
pub enum GlobalSlot {
	// First access runs the function at fn_idx (a zero-arity, no-captures thunk
	// that returns the def's value).
	Pending(u32),
	Evaluating,
	Evaluated(crate::value::Value),
}
