# IR.md — a shared mid-level IR between the typed AST and codegen

**Status:** planned, not started. Step 1 below is a pure, behavior-preserving
refactor of `codegen`; step 2 (a WASM backend) is sketched at the end as the
payoff it unlocks but is out of scope for now. A native/Cranelift backend is
explicitly *not* planned.

## Goal

Introduce a target-independent **IR** that sits between the typed AST and code
emission, and refactor the existing bytecode codegen to go

```
typed AST ──[lowering: elaboration]──▶ IR ──[bytecode emit]──▶ vm::Program
```

instead of walking the AST straight to `Instruction`s in one pass. The bytecode
VM becomes the *first consumer* of the IR; a future WASM backend becomes a
second consumer that reuses every lowering pass and only swaps the final emit.

## Crate topology

`ir` is a **new sibling crate** in the workspace (alongside `compiler`,
`codegen`, `vm`, …; add it to the root `Cargo.toml` members). It holds the IR
types **and** the lowering pass (typed AST → IR), so it depends on `compiler`
(it needs the typed AST). All AST-walking helpers are private to this crate.

`codegen` is re-pointed to consume the IR: it depends on `ir` + `vm` and **no
longer on `compiler::ast`**. The emitter functions see only `ir::IrProgram`,
never the AST — which is exactly the seam a separate crate enforces (it *can't*
reach AST-walking helpers; they don't exist in its dependency set). The public
`codegen::compile(&Compiler) -> vm::Program` stays as a thin orchestrator that
calls `ir::lower(&compiler)` then emits, so `cli` is unchanged. A future `wasm`
crate is a second sibling depending on `ir`, parallel to `codegen`.

```
compiler (typed AST) ──▶ ir (lower → IR) ──┬──▶ codegen ──▶ vm::Program   (today)
                                           └──▶ wasm    ──▶ .wasm          (step 2)
```

Step 1 must change **no observable behavior**: `just test` (both `tests/run` and
`tests/analyze`) passes with zero snapshot diffs, and `bench` shows no
meaningful regression. The bytecode may change shape; the program's
status/stdout/stderr may not.

## Why do this (with one backend, for now)

The justification isn't "three backends" — step 3 is dropped. It's two things:

1. **It's the only way a WASM backend reuses the hard parts.** Closure
   conversion, dictionary passing, pattern-match compilation, and `defer` edge
   insertion are non-trivial and currently fused into bytecode emission. Without
   an IR seam, the WASM backend would reimplement all of it. With the seam, WASM
   is "add an emitter."
2. **It makes the deferred `PERF-NOTES` optimizations tractable.** Decision-tree
   pattern compilation, record-slot lowering (`GetField` → `GetSlot`, no
   hashing), and peephole/const-fold/dead-`Pop` passes are awkward on a fused
   AST→bytecode walk and natural as IR→IR passes. These are *behavior-preserving*
   (they change bytecode, not output), so they can land right after the refactor
   and show up only in `bench`. The IR earns its keep for the VM alone, before
   any WASM work.

## Current state (what we're refactoring)

`codegen/src/emit.rs` (one file, ~3,100 lines) is a single-pass `CodeGen` that:

- runs two pre-passes over all modules: **reserve a global slot** per top-level
  def/alias/instance, then **emit a thunk** per def;
- walks each def body with `emit_expr_with_parents` (~670 lines), emitting
  `Instruction`s into a `FunctionBuilder` as it goes;
- interleaves the elaboration we want to lift out:
  - **identifier resolution** — `resolve_identifier`, `Scope` locals/captures;
  - **closure conversion** — `emit_fun`, explicit capture lists, `alloc_slot`;
  - **dictionary elaboration** — `emit_dispatch_load`,
    `emit_constrained_value_ref`, `emit_resolved_load`, `synthetic_dict_name`,
    `undrained_dispatch_cells`; trait instances are pre-evaluated
    `Value::MethodDict` globals named `<trait>@<head>` (`numeric@int`, …);
  - **pattern lowering** — `emit_pattern`, `emit_sub_patterns_with_cleanup`
    (currently naive linear match, per `PERF-NOTES`);
  - **control flow** — `emit_if`, `emit_when`, `emit_while`;
  - **records/variants** — `emit_field_access`, `emit_variant_construction`;
  - **async** — `body_is_async`/`expr_is_async` mark a def async;
    `compile_constrained_thunk` emits `MakeAsyncClosure`; awaiting relies on the
    VM's frame-snapshot runtime (`vm/src/task.rs`).

`vm::Program` is `{ functions, constants, bytes_constants, regex_patterns,
globals, field_lists, global_by_name, enum_variants, entry, test_suites,
test_new }`. The constants/bytes/regex/field-list pools and `global_by_name` are
**VM-encoding details**; `enum_variants` and the global-name→id allocation are
**target-independent**.

## The IR (deliberately minimal for step 1)

Two passes from the full backend-neutral vision are **explicitly deferred to
step 2**, because the VM doesn't need them and building them now would be
speculative:

- **No `Repr`/unboxing.** The VM is uniformly boxed (`Value`), so the step-1 IR
  is uniformly boxed too — every value is a heap reference. WASM is what wants
  `int`→i64 unboxing; the `Repr` annotations + boxing-coercion pass arrive with
  it.
- **No CPS/state-machine transform.** Async stays exactly as today: a
  `Function.is_async` flag drives `MakeAsyncClosure`, and awaiting rides the
  VM's frame snapshots. The IR keeps `await` as an explicit node; step 2 adds an
  IR→IR pass that rewrites async functions into state machines (the snapshot
  trick can't port to WASM, but that's step 2's problem, not the IR's shape
  today).

So step 1 builds the *seam* and the minimal node set that hosts the current
elaboration — designed so the two deferred passes slot in later (the `is_async`
flag and the `await` node are the anticipated growth points), but not built
speculatively.

Sketch (the `ir` crate's public surface):

```rust
pub struct IrProgram {
    pub functions: Vec<Function>,
    pub globals:   Vec<GlobalInit>,                 // pre-evaluated or thunk-backed
    pub enums:     HashMap<String, Vec<(String, usize)>>, // target-independent
    pub entry:     FuncId,
    pub test_suites: Vec<(String, GlobalId)>,
}

pub struct Function {
    pub name: String, pub module: String,
    pub params:   Vec<VarId>,
    pub captures: Vec<VarId>,   // explicit, from closure conversion
    pub is_async: bool,         // drives MakeAsyncClosure; step-2 CPS seam
    pub body:     Block,
}

pub struct Block(pub Vec<Stmt>);

pub enum Stmt {
    Let(VarId, Rvalue),
    If(Atom, Block, Block),
    Switch { scrutinee: Atom, arms: Vec<(i64, Block)>, default: Box<Block> },
    Loop(Block), Break, Continue,
    Return(Atom),
    Discard(Rvalue),            // effectful rvalue, result dropped
    RunDefer(DeferId),          // emitted on each exit edge
}

pub enum Atom { Var(VarId), Int(i64), Float(f64), Bool(bool), Str(String), Unit }

pub enum Rvalue {
    Use(Atom),
    Bin(BinOp, Atom, Atom),                 // existing monomorphic ops: AddInt, LtFloat, …
    Call(Callee, Vec<Atom>),                // statically-known target
    CallClosure(Atom, Vec<Atom>),
    GetDictMethod(Atom, u32),               // trait-dict slot → callable
    MakeClosure(FuncId, Vec<Atom>),
    MakeRecord(Vec<(Field, Atom)>), GetField(Atom, Field),
    MakeVariant(EnumId, Tag, Vec<Atom>), GetTag(Atom), GetPayload(Atom, u32),
    MakeList(Vec<ListItem>), MakeTuple(Vec<Atom>),
    GlobalRef(GlobalId), Builtin(BuiltinTag),
    Await(Atom),                            // explicit; step-2 pass rewrites these
}
```

Two commitments baked in (both matter for the eventual WASM consumer):

- **ANF** — every intermediate is a `Let`-bound `VarId`; call arguments are
  atoms. Trivial to produce from a functional language, trivial to emit from.
- **Structured control flow is preserved** (`If`/`Switch`/`Loop`, not gotos) —
  WASM requires it, and the bytecode emitter is happy with it.

**Storage is the backend's job, not the IR's.** Lowering emits abstract
`VarId`s; the bytecode emitter assigns them VM stack slots (`base + slot`). A
WASM emitter would map the same `VarId`s to WASM locals. Likewise `captures` is
an abstract list; each backend realizes it its own way.

## What moves where

**Lowering (AST → IR)** absorbs the elaboration:

| Current `emit.rs` | Becomes |
|---|---|
| `resolve_identifier`, `Scope` | `GlobalRef` / `Atom::Var` / capture refs |
| `emit_fun` | a `Function` + `MakeClosure` w/ explicit captures |
| `emit_call`, `emit_dispatch_load`, `emit_constrained_*`, `emit_resolved_load` | dict params + `Call`/`CallClosure`/`GetDictMethod` |
| `emit_if`, `emit_when`, `emit_pattern`, `emit_sub_patterns_with_cleanup` | `If`/`Switch` + `GetTag`/`GetPayload` + binding `Let`s |
| `emit_while` | `Loop`/`Break` |
| `emit_field_access`, `emit_variant_construction` | `GetField`, `MakeVariant` |
| `emit_scope` | scope-kernel calls (unchanged shape) |
| `body_is_async`, `compile_constrained_thunk` | `Function.is_async` + `Await` |

**Bytecode emitter (IR → `vm::Program`)** keeps the VM-specific machinery:
`FunctionBuilder`, `alloc_slot`, jump patching, the constants/bytes/regex/
field-list pools, and `MakeAsyncClosure` emission (driven by `is_async`). It's a
mechanical pass — because the IR is ANF + structured, each `Rvalue`/`Stmt` emits
a local, fixed sequence of instructions.

**Shared up front** (target-independent): the two pre-passes that allocate
global slots and collect `enum_variants`. These run before lowering and feed
both it and the emitter.

## Phasing

Land incrementally so no branch lives long; each phase is mergeable.

- **1.0 — Scaffolding.** Create the `ir` crate (workspace member, deps on
  `compiler`) with the IR types. Extract the global-slot reservation and
  `enum_variants` collection into it. Dead code until wired; no behavior change.
- **1.1 — Lowering (AST → IR).** Port the elaboration from the `emit_*` walk
  into the `ir` crate's `lower` pass, function-by-function mirroring the table
  above. End state: `ir::lower` exists; nothing consumes it yet.
- **1.2 — Bytecode emitter (IR → Program).** Re-point `codegen` to consume
  `ir::IrProgram`, dropping its `compiler::ast` dependency. New mechanical pass
  reusing `FunctionBuilder`/slots/pools. Now `vm::Program` is reachable by two
  paths: the old fused walk and the new IR path.
- **1.3 — Validation & cutover.** Point `codegen::compile` at the IR path,
  delete the old walk once green (below).

## Validation

The acceptance gate is **zero snapshot diffs**: switch `codegen::compile` to the
IR path and run `just test` — `tests/run` (behavior) and `tests/analyze`
(unaffected — it's frontend-only) must be unchanged, and `bench` must not
regress meaningfully.

Stronger transitional check (optional, to gate 1.3): a differential harness that
compiles every fixture *both* ways and runs each `Program` through the VM,
asserting identical status/stdout/stderr. This catches drift in the
behavior-sensitive corners — async (`tests/run/task-*`, `scope-*`), dict
dispatch (`trait-fn-as-value`), pattern cleanup, and `defer` — before the old
path is removed.

## Risks

- **Behavioral drift in edge cases** (async, dict dispatch, pattern/`defer`
  cleanup). → Mitigated by the differential run-output harness across all
  fixtures; these corners have dedicated `tests/run` fixtures already.
- **Scope creep into optimization.** Zero-behavior-change is the hard line for
  step 1; decision trees / record slots / peephole are follow-ons (next
  section), not part of the refactor.
- **The IR shape turning out wrong for WASM.** Accepted: the step-1 IR is
  intentionally minimal and will grow when WASM needs `Repr` and the CPS pass.
  The durable asset is the *seam*, not the exact node set.
- **Long-lived branch / huge diff.** Mitigated by the 1.0→1.2 phasing: IR and
  lowering land as (initially dead) code, the emitter lands next, cutover is the
  small last step.

## Immediately unlocked (behavior-preserving VM wins)

Once the IR path is live, these `PERF-NOTES` items become IR→IR passes that
change bytecode but not output (so they still pass `tests/run`, and show only in
`bench`):

- **decision-tree pattern compilation** (`PERF-NOTES` "Bytecode") — share
  discriminant prefixes across `when` arms;
- **record-slot lowering** — `GetField` by static slot index, no `HashMap`
  hashing;
- **peephole / const-fold / dead-`Pop`** passes over the instruction stream.

---

## Future: Step 2 — a WASM backend (what step 1 unlocks)

Out of scope here; recorded so step 1's IR is designed toward it. A WASM backend
consumes the *same* IR and reuses every lowering pass above. It adds:

- a **`Repr` + unboxing pass** (IR→IR): annotate monomorphic `int`/`float`/`bool`
  as native i64/f64/i32, insert `Box`/`Unbox` coercions at polymorphic
  boundaries (uniform-boxed for generics first; monomorphization later);
- an **async state-machine pass** (IR→IR): rewrite `is_async` functions + `Await`
  nodes into explicit state structs + a step function with a `Switch` on the
  resume point — the portable replacement for the VM's frame-snapshot runtime,
  which cannot port to WASM;
- **WasmGC layout**: records/variants/closures → GC structs (the engine provides
  GC for free), `Switch` → `br_table`, tail calls → `return_call`;
- **host glue**: stdlib builtins → imports (`JSON`, `RegExp`, `crypto`, `fetch`,
  `setTimeout`) or linked WASM libs; and, on top, a DOM-FFI boundary + a pure
  Pluma VDOM/`diff` + an Elm-style `update`/`view` loop (`command msg` ≈
  `task msg`, reusing the existing structured-concurrency runtime).

The VM and WASM backends share everything above the emit line — closure
conversion, dictionary passing, pattern compilation, `defer` edges, and (once
added) the unboxing and state-machine passes. That sharing is the whole point of
step 1.
