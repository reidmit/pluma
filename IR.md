# IR.md — the mid-level IR between the typed AST and codegen

**Status:** the IR seam and the whole Repr/unboxing track are **built and are the
default backend**. The **async CPS state-machine pass** is now built for **every
control-flow shape lowering produces** — `If`/`Switch`/`Match`/`Loop`/`Break`/
`Continue`/`Return`/`Await` plus `defer` — and validated behavior-neutral against
a VM poll-driver. (Building loop support also drove a language change: `await`
inside a `while` loop is now expressible — see "Async CPS" below.) The **WASM
backend** (the third backend) is now **started** — see "WASM backend" below: a
non-async core (primitives, control flow via `Match`, closures, variants,
lazy thunk-globals) runs end-to-end through wasmtime, diffed byte-identical to
the VM. A native/Cranelift backend is explicitly *not* planned.

This file is the design-of-record and the forward plan. For the blow-by-blow of
what shipped (and why), see the `project_fullstack_ir_plan` and
`project_wasm_backend` memories; this doc keeps the *current architecture* and
the *unbuilt roadmap*.

```
typed AST ──[ir::lower: elaboration]──▶ IR ──┬──[codegen::from_ir]──▶ vm::Program   (default)
                                             └──[wasm::emit]────────▶ .wasm          (started)
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

**Async CPS state-machine pass (done; `ir/src/cps.rs`; inert on the VM).**
`cps_transform` rewrites each `is_async` function into poll form so suspension
carries its live state as a value instead of the VM's frame snapshot (the
snapshot can't port to WASM). The original function is left in place (callers
unchanged); the pass generates a sibling `f@poll(state, resume) -> __poll` and
sets `f.poll_fn = Some(it)`. The transform flattens the body's structured
control flow into a CFG of basic blocks (split at each `Await`), computes
liveness across each suspension, and emits a **flat dispatch loop** —
`Loop { Match __tag { 0 => …, _ => … } }`, the structured encoding of a CFG and
exactly the WASM `loop`+`br_table` shape. Each block's terminator either returns
(`ready(v)` / `pending(sub, state')`) or sets the `pc` and falls through so the
loop re-dispatches; only vars live across a suspension ride in the state record
(`__v{id}`; params seeded by the driver as `__a{i}`). The VM's poll-driver
(`vm::task::drive_poll`) advances a transformed function by *calling* its poll fn
— both drivers share the one scheduler. **It now covers every shape lowering
produces:**
- **`defer`** — the live cleanup closures ride in the state as a `__defers` list
  (a fixed field so the driver can find it), appended by each `PushDefer` and run
  LIFO by the driver on completion (`ready(value, defers)`), failure (the
  err-walk), and cancellation (`reap_fiber`) — mirroring the Await-style frame's
  `cleanups`.
- **`Loop`/`Break`/`Continue`** — a source loop becomes a CFG back-edge:
  `Continue`/fall-out set `pc` to the loop header, `Break` to the exit. So an
  `await` inside a `while` splits the loop into resume segments, and the liveness
  fixpoint (already cyclic) threads the loop-carried vars across each suspension.
  This required a **language change**: a task `try` is now type-transparent to
  its continuation, so it can sit in a `nothing`-typed loop body — `await` inside
  a `while` is now expressible (it used to force the body to be a task and so was
  rejected). Soundness: a function that awaits must still return a task (its tail
  is `task.return …`), enforced by tying the enclosing fun's tail to a task at
  each `try`. See `analyzer.rs::do_try_dispatch` + the `dispatch_try_in_expr`
  walk threading `enclosing_tail`.

Every async fn in the corpus transforms; validated byte-identical vs the
Await-style driver by `tests/cps.rs` (completion/failure/cancellation defers,
nested-control-flow awaits, and `await`-in-loop with both discarded and bound
results).

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

### Async CPS state-machine pass — done

The pass now handles every control-flow shape (`defer`, loops, and all the
acyclic forms — see "What's built"), validated VM-anchored by `tests/cps.rs`
(the poll-driver runs the transformed corpus to byte-identical output vs the
Await-style driver). Nothing remains here before the WASM backend consumes it.

### WASM backend (the payoff) — started

The `wasm` crate (sibling to `codegen`) consumes the *same* IR and reuses every
pass above; `wasm::emit(&IrProgram) -> Result<Vec<u8>, Diagnostics>` swaps only
the emit. Validated by `tests/wasm_diff.rs` — a clone of `ir_diff` that runs each
emitted module in **wasmtime** (a Rust dev-dependency, GC + tail-call enabled)
with Rust host glue, diffed byte-identical to the VM. The allowlist grows
milestone-by-milestone; `wasm_coverage_report` (ignored) scans the whole corpus.

**Built (non-async core).** Pipeline: `ir::lower` → `resolve_direct_calls` →
uniform `repr::insert_coercions` → reachability DCE (the prune is load-bearing —
even `print (1+2)` lowers the whole prelude) → emit. WasmGC layout: a `$value`
supertype struct with an `i32` tag, scalar subtypes (`$int`/`$float`/`$bool`/
`$str`+`$bytes`), `$valarray`, `$closure`, `$variant`, `$ctor`. Uniform-boxed
contract → arity-keyed function types with an **implicit closure-env param 0**.
Covered: all int/float arithmetic + split comparisons + `Not`; `Box`/`Unbox`
(struct.new / ref.cast+struct.get); `Match` (Wildcard/Bind/Literal/Variant) +
`If`/`Loop`/`Break`/`Continue` as structured `block`/`loop`/`br`; closures
(`MakeClosure` → `$closure` + a funcref table, `CallClosure`/`TailCall` →
`call_indirect`/`return_call_indirect`, captures read from env in a prologue);
variants (`MakeVariant`/`MakeVariantCtor`/`GetTag`/`GetPayload`, ctor-application
shortcut); builtin calls (`print` host import; the `GlobalRef`-to-builtin →
host-call rewrite); lazy thunk-globals (cached value + i32 init flag);
**structural `Eq`/`Ne`** (a synthetic `__eq` runtime fn — recursive over variants,
byte-loop for strings, IEEE float); **trait dicts** (`$methoddict` +
`MakeDict`/`GetDictMethod`; prelude builtin-method dicts realized via per-builtin
*wrapper* functions — pure-compute bodies for `int/float` `+-*/`/`negate` and
`int/float-compare` — wrapped in capture-free `$closure`s, built lazily). ~29
fixtures green incl. `factorial`/`recursion`/`generic-enum`/`comparison-ops`/
`closures`/`cross-module`/`unary-minus`/`partial-application`.

A zero-arg-closure arity quirk is also handled: `fun { body }` lowers with zero IR
params but is typed `nothing -> a` (arity 1 — its call sites pass the `()` arg), which
`call_indirect` would trap on; such `MakeClosure` targets get a phantom wasm param so
their type matches callers (unblocks `??`/`or-else`: `coalesce-*`).

**Tuples/lists/records** are built: `$tuple`/`$list` (`{tag, $valarray}`) and `$record`
(`{tag, name-sorted $valarray names, parallel values}`); `MakeTuple`/`MakeList`
(element-only)/`MakeRecord`/`GetField` + tuple/list/record `Match` patterns + their
structural-`Eq` arms. Field access is a `__getfield(rec, name)` runtime helper (name
scan via `__eq`); a list `...rest` binding is a `__list_tail(list, n)` helper. ~39
fixtures green (incl. `equality-structural`, `swap-tuple`, `record-pattern`).

List **spread** (`[a, ...xs, b]`, via `__arrconcat`) and `RecordUpdate` (via
`__record_update`) are built; so is the full **`to-string`** / **`Concat`** /
**`Interpolate`** stack — a wasm-native `__tostring` covering scalars, strings,
`__int_str` decimal formatting, **compounds** (tuple/list/record/variant, formatted
recursively by folding byte arrays with `__bytesconcat`), and **floats** (delegated to
a `float_to_str` host import that writes into a caller-passed GC `$bytes` buffer — as a
browser target would delegate to JS). **Variant printing** works (`$variant` carries a
display-name field). A returned `err` result becomes a `runtime error` exit (checked by
the runtime, mirroring `vm::VM::run`).

An **inline-builtin** path (`is_inline_builtin` + `FnEmitter::inline_builtin`) emits
pure-compute leaves directly over the `$value` GC layout — no host import, no synthetic
helper — starting with `list-get` (array.get) and `list-length` (array.len → boxed
`$int`). This is the seam that lets the **builtin surface grow two ways**: inline-emit a
pure leaf in wasm, *or* rewrite the stdlib def in pure Pluma so it lowers through the
existing pipeline with no new emit code (and runs on the VM unchanged). The
stdlib-to-Pluma rewrite took that second path for most of `core.list` plus
`option.then`/`result.then`/`assert.*`. The list *consumers* (`fold`/`each`/`contains`/
`find`/`any`/`all`/`head`) recurse by index over `length`+`list-get` (NOT `[h,...rest]`
recursion, which copies the tail and is O(n²)); the list *builders* (`map`/`take`/`drop`/
`reverse`/`filter`) build via two tabulate primitives — `list-build` (`fun int -> a`)
and `list-collect` (`fun int -> option a`, compacts), emitted as **synthetic helpers
that `call_indirect` the closure** (the first such helpers). `try`-over-option/result
now works on wasm because it desugars to the now-pure `then`. **82 fixtures green, 0
diffs.** (The differential harness runs wasmtime's **null collector** — the `gc-null`
feature — because wasmtime 30's deferred-ref-counting collector panics on a valid module
once a real GC runs.)

**Still unbuilt.** The broad **builtin host surface** (`string.*`, `math.*`, `dict.*`,
`bytes.*` — ~197 tags, M7 — gates most remaining `string-*`/`core-*` fixtures;
`list.sort` stays a Rust builtin because functional merge-sort over the array backing is
O(n²) per merge without a mutable buffer, and `string.join`/`split` stay Rust because
string building is its own O(n²) problem — a byte-buffer builder, separate from the list
tabulators); `string/bytes-compare` + hash wrappers; record
`{...rest}` named-rest binding; `Switch`→`br_table`; `Const` globals; async (run
`cps_transform`, then reimplement the fiber/scope/timer scheduler — M9). DOM/FFI/VDOM
out of scope.

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
