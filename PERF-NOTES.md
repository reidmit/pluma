# Performance compromises

A running list of decisions where we picked simplicity over performance, to
revisit when profiling shows it matters.

## Value representation

- **Heap-allocated compound values via `Rc<T>`.** Tuples, lists, records,
  variants, closures, and regexes all live behind an `Rc` so the `Value`
  enum stays at 16 bytes (8-byte payload + 1-byte tag). Cloning a Value is
  a refcount bump (fast). Building a new compound allocates.
  *Future:* NaN-boxing would get us to 8 bytes per Value but requires
  unsafe code and care with NaN propagation. Defer until a profile shows
  Value cloning or stack-slot copying as a hot path.

- **`Rc<String>` for interned-ish strings.** String constants from the
  source program go through the constants pool as `Rc<String>`, sharing the
  allocation across all uses. Dynamic strings (from `to-string`,
  interpolation) allocate fresh.
  *Future:* a real string interner that hashes constructed strings could
  reduce duplication for programs that build many identical strings.

## Stack layout

- ~~Per-frame `Vec<Value>` for locals.~~ **Done.** Unified stack with
  `base + slot` indexing now. Frames just hold offsets. Benchmark wins
  in the 15-24% range on call-heavy code (fib, record-access, sum-list).
  Tail-recursive code didn't improve (no frame allocations to save).

## Bytecode

- **Naive pattern-match dispatch.** Each `when` case is a linear sequence
  of "match this pattern; on failure jump to next case". Works correctly,
  but redoes shared discriminant tests when multiple cases share a prefix
  (e.g. all checking the same enum tag).
  *Future:* decision-tree compilation for `when` — share common prefixes
  and minimize tests. Standard compiler optimization, well-documented in
  "Compiling Pattern Matching to Good Decision Trees" (Maranget 2008).

- **No instruction operand packing.** Each `Instruction` is a Rust enum
  variant carrying its operands inline (e.g. `LoadInt(i64)`). Wider than
  necessary but simple — no separate constants table for small immediates.
  *Future:* a more compact bytecode representation (u32 instructions with
  operand tables) would shrink memory footprint and improve cache locality.

- **No instruction-stream optimization passes.** Codegen emits straight
  from AST to instructions, no peephole/dead-code/constant-folding passes.
  *Future:* obvious wins like collapsing consecutive `Pop` ops, eliminating
  redundant moves, evaluating constant arithmetic at compile time.

- **Hash-map field lookups for records.** `GetField` does an `HashMap::get`
  on a `Rc<HashMap<String, Value>>`.
  *Future:* record types could be lowered to fixed-shape tuples with a
  separate static field-index table. `GetField` becomes `GetSlot(usize)`,
  no hashing.

## Dispatch

- **Standard match-based dispatch loop.** Stable Rust can't do computed
  goto / threaded code, so we get whatever `match` lowers to.
  *Future:* on nightly or via specific build tricks, threaded code can
  give 10-30% on tight loops. Not worth it until everything else is tuned.

## Cross-cutting

- **No JIT.** We're an interpreter. Even with all the above, a JIT (e.g.
  via Cranelift) would dwarf any constant-factor improvement we get from
  micro-optimizing the bytecode VM.
  *Future:* once the bytecode shape is stable, a Cranelift-backed JIT
  for hot functions is a natural next step.
