# IR.md — the mid-level IR between the typed AST and codegen

**Status:** the IR seam and the whole Repr/unboxing track are **built and are the
default backend**. What remains is the **async CPS state-machine pass** and the
**WASM backend** that consumes it all. A native/Cranelift backend is explicitly
*not* planned.

This file is the design-of-record and the forward plan. For the blow-by-blow of
what shipped (and why), see the `project_fullstack_ir_plan` memory; this doc
keeps the *current architecture* and the *unbuilt roadmap*.

```
typed AST ──[ir::lower: elaboration]──▶ IR ──┬──[codegen::from_ir]──▶ vm::Program   (today, default)
                                             └──[wasm emit]─────────▶ .wasm          (the payoff, unbuilt)
```

The bytecode VM is the *first consumer* of the IR; the WASM backend will be a
second consumer that reuses every lowering and IR→IR pass and only swaps the
final emit. That reuse — closure conversion, dictionary passing, pattern
compilation, `defer` edges, unboxing, and (once built) the CPS transform shared
above the emit line — is the entire reason the IR exists.

## Crate topology (as built)

- **`ir`** — IR types (`types.rs`) + the lowering pass `lower(&Compiler) ->
  IrProgram` (`lower.rs`) + the IR→IR passes (`repr.rs`, `resolve.rs`,
  `mono.rs`). Depends on `compiler` (needs the typed AST). All AST-walking is
  private to lowering; the IR itself is type-free.
- **`codegen`** — `from_ir.rs` lowers `ir::IrProgram → vm::Program` (the live
  default). `emit.rs` is the legacy fused AST→bytecode walk, **kept on purpose**
  as (a) the differential harness's live oracle, (b) the `--backend ast` /
  `PLUMA_BACKEND=ast` fallback, and (c) a stable baseline for the future WASM
  backend. `codegen::compile` selects the backend; the default is IR.
- **`vm`** — unchanged consumer of `vm::Program`.
- A future **`wasm`** crate is a second sibling depending on `ir`, parallel to
  `codegen`.

## The IR (as built)

Two commitments, both for the eventual WASM consumer:

- **ANF** — every intermediate is a `Let`-bound `VarId`; call args are `Atom`s.
- **Structured control flow** — `If`/`Switch`/`Match`/`Loop`, never gotos (WASM
  requires it; the bytecode emitter is happy with it).

**Storage is the backend's job.** Lowering emits abstract `VarId`s; `from_ir`
maps them to VM stack slots, a WASM backend would map them to locals. `captures`
is an abstract list each backend realizes its own way.

The actual node set lives in `ir/src/types.rs`; the shape worth knowing here:

- `IrProgram { functions, globals, enums, entry, test_suites, test_new }`.
- `Function { name, module, params, captures, is_async, body, var_reprs,
  param_reprs, ret_repr }`. The last three are the Repr track (below).
- `StmtKind`: `Let`, `If`, `Switch`, `Match` (pattern-level, not pre-compiled to
  a decision tree), `Loop`/`Break`/`Continue`, `Return`, `Discard`, plus the
  `defer` edges `RunDefer`/`PushDefer`. Every `Stmt` carries a source `Range` for
  error attribution.
- `Rvalue`: `Use`, `Bin`, `Not`, `Call(Callee, …)` / `CallClosure` / `TailCall`,
  the dict machinery `GetDictMethod`/`MakeDict`, `MakeClosure`, records/variants
  (`MakeRecord`/`GetField`/`MakeVariant`/`MakeVariantCtor`/`GetTag`/`GetPayload`),
  `MakeList`/`MakeTuple`, `Interpolate`, `Regex`, `GlobalRef`, `Builtin`,
  `Await`, and the Repr coercions `Box`/`Unbox`.
- `Callee { Function(FuncId), Global(GlobalId), Builtin(String) }` — `Function`
  is a resolved direct call (see resolution, below).
- `BinOp`: arithmetic split by type (`AddInt`/`AddFloat`/…); `Concat`, `And`,
  `Or`, structural `Eq`/`Ne`; and comparisons **split by operand repr**
  (`LtI64`/`LtF64`/`LeI64`/… ×4 relations) so the op self-describes its operand
  repr for WASM. `from_ir` maps each back to the VM's one polymorphic opcode.

## What's built

**Step 1 — the seam (done; IR is default).** Lowering ports all the elaboration
that was fused into `emit.rs`: identifier resolution, closure conversion,
dictionary elaboration (trait constraints → dict params + `GetDictMethod`;
instances → pre-evaluated `MethodDict` globals), pattern compilation, `defer`
edges, async marking. Differential harness `tests/ir_diff.rs` compiles every
fixture *both* ways and asserts identical run output. Perf reached parity then
surpassed AST via an operand-stack peephole in `from_ir` and `TailCall` lowering.

**Devirtualization (done).** Concrete `int`/`float` `+ - * /` and `< <= > >=`
emit direct `BinOp`s instead of dispatching through the boxed `numeric`/`ord`
method dict. Done *in lowering* (it's type-directed and the IR is type-free, so a
post-lowering pass would need Repr first; lowering still has `expr.ty`). IR is
~2× faster than AST on arithmetic-heavy code as a result. (Also drove the
language change: concrete float relations now follow IEEE-754, NaN→false.)

**Repr / unboxing pass (done; `ir/src/repr.rs`; inert on the VM).** The WASM
prerequisite that makes boxing explicit. `repr_of_type` projects a `compiler`
type to `Repr {Boxed,I64,F64,I32}` (the single bridge from types into the
type-free IR). `infer_reprs` assigns each `VarId` a repr; `insert_coercions`
splices `Box`/`Unbox` at repr mismatches; `validate_reprs` is the WASM-readiness
checker. `if`/`when` **join vars take their arms' unified repr** (all arms agree
→ that repr, else `Boxed`). All inert on the VM (`Box`/`Unbox` lower to a no-op;
split comparisons map to the one VM opcode).

**Param/return monomorphization (done; inert on the VM).** Gives eligible
concrete functions an unboxed calling convention so caller↔callee chains pass
i64/f64/i32 with no box/unbox churn. Three pieces:
- **Direct-call resolution** (`resolve.rs`) — the enabling pass. A top-level call
  lowers to `CallClosure(GlobalRef(g))`, hiding the callee; when `g`'s thunk is a
  capture-free non-async closure of `fid`, rewrite to `Call(Callee::Function(fid))`
  and prune the dead `GlobalRef`. Makes the callee visible at the call site.
- **Eligibility + escape analysis** (`mono.rs`) — keep an unboxed signature only
  for a function that is a non-escaping (no surviving `GlobalRef`), concrete,
  **self-recursive** top-level def with an unboxed param; everyone else reverts to
  all-`Boxed`. The self-recursive + unboxed-param rule is a cheap **profitability
  proxy**: monomorphization relocates coercions to call boundaries, so it only
  pays when an unboxed value rides the recursion.
- **Interprocedural Repr** — `repr.rs` is parametrized by `Sigs` (`uniform()` =
  the old all-boxed contract used by the default VM path; `from_program` makes
  `infer_reprs`/`insert_coercions`/`validate_reprs` honor each callee's signature).

## Validation philosophy (VM-anchored)

There is no WASM backend yet, so nothing *consumes* the unboxed reprs — they're
validated-but-unused. Every Repr/mono slice is therefore **inert on the VM by
design** and anchored by three VM-checkable properties instead of a real
consumer (`tests/ir_repr.rs`, `tests/ir_mono.rs`):

1. **Behavior neutrality** — the transformed program runs to byte-identical
   output (a bad coercion would fault and diverge).
2. **A static validator** — `validate_reprs` proves no naked cross-repr flow
   remains (the discipline a WASM emitter will rely on).
3. **Non-vacuity** — the pass demonstrably does work (inserts coercions,
   resolves calls, monomorphizes functions) and, for mono, **never increases
   corpus-wide coercions** (the profitability invariant).

When the WASM backend lands it becomes the real oracle; until then, keep new
IR-track work to this contract (no speculative dead code without a VM anchor).

## Roadmap

### Async CPS state-machine pass (the next prerequisite)

Today async rides the VM's frame-snapshot runtime: `Function.is_async` drives
`MakeAsyncClosure`, and `Await` suspends by snapshotting the whole frame
(`vm/src/task.rs`). That snapshot trick **cannot port to WASM** (no access to the
native stack). The pass: rewrite each `is_async` function + its `Await` nodes
into an explicit state machine — a state struct holding the live vars across each
suspension point, plus a step function with a `Switch` on the resume point.
`is_async` + the explicit `Await` node are the growth points already in the IR.

VM anchor (same principle as above): add a poll-style VM task driver so the
state-machine output runs the existing `task-*`/`scope-*` differential harness to
identical output before any WASM exists. Mirror a `tests/run/task-*` fixture.

### WASM backend (the payoff)

Consumes the *same* IR and reuses every pass above; adds only the emit:

- **Reprs are already in place** — `param_reprs`/`ret_repr`/`var_reprs` +
  `Box`/`Unbox` + split comparison ops give the emitter i64/f64/i32 locals and
  GC-ref boxing boundaries to read straight off. This is the work the Repr/mono
  track front-loaded.
- **WasmGC layout** — records/variants/closures → GC structs; `Switch` →
  `br_table`; tail calls → `return_call`.
- **Host glue** — stdlib builtins → imports (`JSON`, `RegExp`, `crypto`, `fetch`,
  `setTimeout`) or linked WASM libs; DOM via a host-FFI boundary (`externref`),
  then a pure-Pluma VDOM/`diff` + Elm-style `update`/`view` loop (`command msg` ≈
  `task msg`, reusing the structured-concurrency runtime).

### Repr/mono follow-ons (when the WASM backend makes them measurable)

- **Profitability cost model / call-graph fixpoint** — replace the self-recursive
  proxy with a real model so mutual recursion and unboxed pipelines also qualify
  (and nothing that doesn't pay does).
- **Direct-tailcall resolution** — `resolve.rs` only resolves `CallClosure`, not
  `TailCall` (no direct-tailcall IR form yet), so tail-recursive numeric loops
  are left ineligible. Adding it unlocks them for both the indirection skip and
  monomorphization.
- **Boxed unbox-call-rebox wrappers** for functions that both escape *and* want a
  specialized signature (today: escape ⇒ stay boxed).
- **Generic specialization** per concrete instantiation (template-style), beyond
  the uniform-boxed generic fallback.
- **Unbox more ops** — `Eq`/`Ne` (structural, currently boxed), `GetTag`/
  `GetPayload` (int-ish, currently boxed); `negate` devirt (needs a unary IR
  node).
- Wiring `resolve_direct_calls` into the **default VM path** (a closure-
  indirection skip) — only if `bench` justifies it; kept out today to keep the
  default path byte-identical.

### Behavior-preserving VM wins (independent of WASM)

Still-open `PERF-NOTES` items that are natural IR→IR passes (change bytecode, not
output): **decision-tree pattern compilation** (share discriminant prefixes
across `when`/`Match` arms), **record-slot lowering** (`GetField` by static slot
index, no hashing), and **peephole/const-fold/dead-code** passes over the IR.
