# RECORDS.md — record representation on the WASM backend

**Status:** Step 2.0 (type-threading) **done**; Step 2.1 (nominal `$shapeN` reads
via `ref.cast` + `struct.get`) **done**, including record-shape **monomorphization**
so row-polymorphic / record-parameter functions also reach `struct.get`. Tier B
(cast-free, Step 3) and a few narrow residuals (below) are the remaining follow-ons.
This is the design-of-record and forward plan for moving WASM record field access
from a runtime name-scan to the **theoretical floor** — a single (cast +)
`struct.get` at a compile-time-constant index — by giving each record *shape* a
nominal WasmGC struct type.

> **As-built note (supersedes parts of §5–§7 below).** The dual representation
> shipped with a *simpler* coercion model than the lift/lower sketch: a single
> **inline `lift`** (nominal `$shapeN` → uniform `$record`) and **scan-on-uniform
> reads** — no `lower` is needed, because a read site simply name-scans when its
> receiver is uniform. It also needed **no IR changes** for the no-mono stage: the
> whole nominal/uniform decision + lift insertion lives in the WASM backend
> (`wasm/src/scan.rs::compute_nominal` + `wasm/src/emit.rs`). A `MakeRecord` whose
> result is *read as a record locally* (a `GetField` receiver or a record-pattern
> `Match` subject) is built nominal; a nominal value reaching any other position is
> lifted inline by `emit::atom`. This keeps the open/row-poly path (the
> `record-access` benchmark) from regressing without monomorphization (those
> records are never locally read, so they stay uniform — today's path). `lower` and
> the `shape_id` br-table dispatch from §5/§B are unbuilt and only needed for a
> *generic* lift over an unknown shape, which type-directed insertion avoids;
> `shape_id` is still stamped into each `$shapeN` for that future path.
> `RecordRest::Bind` (`...rest`) is implemented on the nominal path.
>
> **Monomorphization (as-built, WASM-side).** `wasm/src/mono.rs` clones a
> candidate function (one whose param is *read as a record* — a `GetField`
> receiver or record-pattern `Match` subject) per call-site shape, where the arg
> is a `MakeRecord` of statically-known shape. The clone's call is rewritten to
> it; the original stays for unknown-shape calls (uniform fallback → never
> regresses). A side map (clone `FuncId` → per-param nominal shapes) is threaded
> into the emitter, which (a) seeds the clone's params nominal, (b) builds a
> `MakeRecord` arg flowing into a nominal param as `$shapeN` and passes it raw (no
> `lift`). This is the WASM-only specialization the plan's §6.D calls for; it does
> **not** touch the IR types or the scalar-repr `mono::monomorphize` (whose
> unboxed signatures the WASM `FuncKind::Pluma(arity)` interner can't yet express
> — that's Tier B). Bounded at 8 clones/source (uniform past the cap).
>
> **Follow-ons (status).**
> - ✅ **Nominal `RecordUpdate`** — `{ ...base, f: v }` on a nominal base builds a
>   `$shapeN` via `struct.new` (copying base's inline fields, substituting
>   overrides) instead of lifting to the uniform `__record_update` helper. Marked
>   nominal (shape-preserving) when the base is nominal and the result is read
>   locally; chains (`a → b → f`). `emit::record_update_nominal`.
> - ✅ **Uniform-path `RecordRest::Bind`** — the last `...rest` gap (a rest on a
>   *uniform* match subject, e.g. a nested inner record bound from a field read) is
>   closed by a `__record_rest(rec, excluded)` helper (`Helper::RecordRest`) that
>   filters `rec`'s fields by name at runtime. `record-pattern-nested-rest` is now
>   in the differential allowlist.
> - ✅ **Microbench** (`tests/wasm_diff.rs::record_access_microbench`, `#[ignore]`):
>   nominal+mono vs forced-uniform (the `PLUMA_WASM_UNIFORM_RECORDS` toggle) on
>   `record-access` → **2.78× speedup** (48.2s vs 133.8s over 200×10000 build+read
>   ops; absolute times inflated by wasmtime's baseline + the drc collector, the
>   *ratio* is the signal).
> - ⏸️ **Tier B (cast-free reads)** — *evaluated and deferred*. The microbench shows
>   Tier A already captures the win by eliminating the O(F·L) name-scan; the
>   residual `ref.cast` is one O(1) RTT check (hoisted/CSE'd across reads of one
>   receiver), not the bottleneck. Tier B (repr-type record locals/params/returns at
>   `(ref $shapeN)` + shape-aware `FuncKind` signatures) is a large cross-cutting
>   change for a marginal cast removal — not worth it now, per the §7 gating ("do it
>   only once … the cast is shown to matter").
> - ⏸️ **Per-field unboxing** (a scalar record field as inline `i64`) — out of scope
>   until/unless Tier B lands.

This is a sub-project of the WASM backend (see `IR.md` "WASM backend" and the
`project_wasm_backend` memory). It changes only the `wasm` crate's record
*representation* plus the IR plumbing that feeds it; the VM backend and language
semantics are unchanged. Every step is validated VM-anchored (zero snapshot diffs
against the bytecode VM), matching the rest of the backend.

File/symbol references below are snapshots and will drift; treat the symbol names
as authoritative, the line numbers as hints.

---

## 1. Goal

A record field read today is **O(F · L)** — a linear scan over the record's
name-sorted field array, comparing each name to the target via `__eq` (a full
string byte-compare), where F = field count and L = key length. We want it to be:

```wat
local.get $r          ;; the record, as (ref null $value)
ref.cast $shapeN      ;; one O(1) RTT check  — eliminable (see Tier B)
struct.get $shapeN k  ;; k is a compile-time constant
```

i.e. **O(1)**, no string compares, and — as a bonus — **one allocation per
record** (fields stored inline) instead of three (the `$record` struct plus a
names array plus a values array).

---

## 2. Current representation (as built)

### 2.1 The value hierarchy

The WASM backend is *not* uniformly boxed the way the VM is: scalars live unboxed
in locals (`int`→i64, `float`→f64, `bool`→i32), and every *boxed* value is a GC
reference to a subtype of `$value`. Crucially, **`$value` (`T_VALUE = 0`) is an
open, non-final supertype** carrying just `{ i32 tag }`, and each concrete kind is
a `struct` subtype of it (`wasm/src/types.rs`, `FuncTypes::encode`). The uniform
boxed type used for params, captures, `$valarray` elements, and every `Boxed`
local is `value_ref()` = `(ref null $value)`.

This is the load-bearing fact for the whole plan: **a nominal per-shape record
struct can itself be declared as a subtype of `$value`**, so it remains storable
in a `$valarray`, passable as a boxed param, comparable by the generic helpers,
etc. — without any new "boxing." We are adding subtypes, not a parallel universe.

### 2.2 The `$record` struct

```
T_RECORD = 13:  struct { i32 tag, (ref $valarray) names, (ref $valarray) values }
TAG_RECORD = 13
```

`names`/`values` are parallel arrays, **sorted by field name** at construction
(`wasm/src/emit.rs`, `Rvalue::MakeRecord` — `sorted.sort_by(|a,b| a.0.cmp(b.0))`).
So a record is self-describing: it carries its own field names at runtime.

### 2.3 The six record touchpoints in the `wasm` crate

| Touchpoint | Where | What it does today |
|---|---|---|
| Construct | `emit.rs` `Rvalue::MakeRecord` | sort by name; build parallel `names`/`values` arrays; `struct.new $record` |
| Field read | `helpers/record.rs` `build_getfield_fn` (`Helper::GetField`) | **linear-scan** `names`, `__eq` each, return parallel `values[i]`; trap if absent |
| Update | `helpers/record.rs` `build_record_update_fn` (`Helper::RecordUpdate`) | copy `values`, overwrite the matched slot, share `names` |
| Pattern match | `emit.rs` `Pattern::Record` in `test_pattern` | per field, call `__getfield` then recurse; `RecordRest::Exact` checks `names.len`; `RecordRest::Bind` is **not yet supported** (pushes a diag) |
| Structural eq | `helpers/eq.rs` (`Helper::Eq`, `TAG_RECORD` arm) | compare the two `values` arrays **positionally** — same Pluma type ⇒ same name-sorted order, so names need not be compared |
| `to-string` | `helpers/tostring.rs` (`TAG_RECORD` arm) | read `names` + `values`, render `{k: v, ...}` |
| `wire` codec | `helpers/wire.rs` (`s_record` arms) | encode: walk `values` positionally in schema (name-sorted) order; decode: build fresh `names`+`values` arrays → `struct.new $record` |

### 2.4 VM contrast (the oracle)

The VM is not actually faster in kind — `Value::Record(Rc<HashMap<String,Value>>)`
(`vm/src/value.rs`), and `Instruction::GetField(idx)` interns the *name* to a
constant-pool index (`codegen/src/from_ir.rs`) but at runtime still does
`HashMap::get(name)` (`vm/src/vm.rs`). Neither backend resolves a field to a fixed
slot today. (The VM remains the behavioral oracle regardless; we are not changing
it.)

---

## 3. Why the representation is shaped this way

The names/values arrays are not incidental. They make a record **self-describing**,
and that property is depended on by two distinct things:

**(a) Generic consumers.** `__eq`, `__tostring`, and the `wire` codec all operate
on a record received as a bare `$value` of statically-unknown *shape* — e.g.
`to-string` applied to a polymorphic value, list/dict equality over boxed
elements, `wire` driven by a runtime schema tree. They walk the value/name arrays
generically; they never need to know the concrete field set at compile time.

**(b) Row polymorphism.** Pluma supports record polymorphism as a tested,
intentional feature. `Type::Record(Vec<(String,Type)>, Option<usize>)`
(`compiler/src/types/type.rs`): a `None` tail is a *closed* record (exactly these
fields); a `Some(var)` tail is *open* ("at least these fields"). A function can be
generic over the row — see `tests/run/record-pattern-row-poly/main.pa`:

```pluma
def get-name = fun r { when r is {name: n, ...} { n } }   # r : Record([(name,_)], Some(ρ))
# ... get-name {name:"reid"} ; get-name {name:"alice", age:30} ; get-name {name:"bob", role:"eng", level:99}

def drop-name = fun r { when r is {name: _, ...rest} { rest } }   # produces a record of unknown shape
# let r1 = drop-name {name:"x", age:28} ; print (to-string r1.age)   # ← uniform value crosses back into closed access
```

`get-name` compiles **once** but is applied to three different shapes; `name` sits
at a different offset in each. The self-describing array lets the single compiled
body scan for `name` regardless. (Other examples: the comparator/projection lambdas
in `compiler/src/stdlib/list.test.pa`, `regex.test.pa`;
`benchmarks/programs/record-access/main.pa`.)

Record usage overall is *moderate* — most stdlib data is lists/dicts/strings — but
row polymorphism genuinely occurs and must keep working.

---

## 4. The floor, and the WasmGC constraint that makes it hard

### 4.1 Two tiers of "floor"

Because the boxed calling convention types every binding as `(ref null $value)`, a
nominal-struct field read needs a `ref.cast` to the concrete shape first:

- **Tier A — near-floor: `ref.cast $shapeN` + `struct.get k`.** The cast is one
  O(1) RTT check, vastly cheaper than the scan, and when several fields are read
  from the same record the cast is hoisted/CSE'd (cast once, `struct.get` many).
  Keeps the uniform calling convention intact.
- **Tier B — true floor: bare `struct.get k`, no cast.** Requires the binding to
  be statically typed `(ref $shapeN)` rather than `$value`, which means the Repr
  system carries concrete record-shape types through locals/params/returns and
  `mono` specializes signatures accordingly (see §7, Step 3).

Either tier also collapses a record to a **single allocation**.

### 4.2 The hard constraint: `struct.get`'s index is a static immediate

WasmGC has **no "get field at a computed offset."** `struct.get`'s field index is
an immediate baked into the instruction. This collides head-on with row
polymorphism:

- In `get-name`'s single compiled body, `name`'s offset varies by caller shape.
  There is no static `k` to emit.
- No subtyping trick rescues this: WasmGC struct subtyping requires a subtype to
  extend its supertype's fields *as a prefix in the same order*. You cannot make a
  given field name land at the same index across arbitrary field sets (structs are
  dense — no holes), so structural rows can't be encoded as a subtype lattice.

So **every** place that needs a record's field/layout but doesn't statically know
the shape — both row-polymorphic user code (b) and the generic consumers (a) —
loses the ability to `struct.get`. Each such site has exactly three escapes:

1. **Monomorphize** the function per concrete shape, so the offset becomes static.
2. **`call_indirect` a per-shape helper** (a vtable selected by a runtime shape id).
3. **Fall back to an array representation** that supports dynamic access — i.e. the
   current self-describing `$record`.

This is the crux of the whole project. The plan below is mostly about *which
escape, where*.

---

## 5. The recommended design: dual representation with coercions

Rather than make the generic consumers dispatch per-shape (escape 2 for all four
of `__eq`/`__tostring`/`wire`-enc/`wire`-dec — a lot of generated code and four
vtables), keep **two representations** and coerce between them at boundaries —
exactly mirroring how `ir/src/repr.rs` already inserts `Box`/`Unbox` coercions for
scalars:

- **Nominal** `$shapeN` — the fast path. Built at `MakeRecord` (the literal's shape
  is always statically known); read/updated/pattern-matched via `ref.cast` +
  `struct.get` wherever the *static type at the site is closed*.
- **Uniform** `$record` (today's names/values rep, **kept as-is**) — the fallback.
  All generic consumers (`__eq`, `__tostring`, `wire`) continue to run on it
  **unchanged**. Row-polymorphic code that can't be monomorphized runs on it too.

A new IR→IR analysis decides, per record-typed value, which representation it
flows in, and inserts:

- **lift** (`nominal → uniform`): materialize the names/values arrays from a
  `$shapeN` when the value reaches a generic consumer or a row-polymorphic
  position. Selected by a **shape id** carried in the struct → `call_indirect` a
  generated per-shape `lift_S` (this is the *one* family of generated per-shape
  helpers we need, instead of four).
- **lower** (`uniform → nominal`): build a `$shapeN` from a uniform `$record` when
  a value of statically-known closed shape arrives in uniform form — e.g. the
  result of `drop-name` (born uniform inside a row-polymorphic body) flowing into
  `r1.age` where `r1`'s type is closed (`tests/run/record-pattern-row-poly`
  line 28-29). Generated per-shape `lower_S`.

Monomorphization (escape 1) is the *first* tool: where `mono` can resolve a
row-polymorphic function's shape at every relevant call site (the observed corpus —
`get-name`, `drop-name`, the projection lambdas — is all direct-called with
statically-known shapes), specialize it so it's born nominal and never touches the
uniform path. The uniform fallback + coercions exist for the **residual** that
can't be monomorphized (record-polymorphic function *values* that are stored,
passed indirectly, or recursive over shape). `log()`/document any such residual so
we know what's paying the coercion.

This bounds the new surface to: a shape registry, a shape-id + `lift_S`/`lower_S`
generated helpers and their dispatch, the IR repr-analysis for record shape, and
the monomorphic emit paths — while leaving the four generic consumers, the schema
machinery, and language semantics untouched.

> **Alternative considered — full nominal + per-shape vtables (escape 2 everywhere):**
> generate `eq_S`/`tostring_S`/`encode_S`/`decode_S` per shape and dispatch all
> generic consumers through shape-id tables. Marginally faster on the generic path
> (no array materialization) but four families of generated helpers + four
> vtables, and it duplicates logic already correct in the uniform helpers. Not
> recommended; the dual-rep coercion approach reaches the same *floor on the hot
> monomorphic path* for far less code and risk.

---

## 6. Work breakdown

**A. Shape registry.** Intern each distinct field-name *set* → a nominal struct
subtype of `$value`: `struct { i32 tag, i32 shape_id, f0, f1, … }`, fields in
name-sorted order, each field `(ref null $value)` (boxed — `repr.rs` already forces
record fields `Boxed`; per-field unboxing is a later optimization). Mirror the
arity-interning pattern in `FuncTypes` (`wasm/src/types.rs`). The registry maps a
sorted name-set → `(struct type index, shape_id)`. Populated by a scan pass over
the IR (cf. `wasm/src/scan.rs`, which already interns record-pattern field names).

**B. Shape id + per-shape coercion helpers.** The `shape_id` field lets a generic
boundary recover the layout of a record it holds only as `$value`. Build, per
registered shape, a `lift_S` (→ uniform `$record`) and `lower_S` (← uniform
`$record`), plus a dispatch (a `br_table`/`call_indirect` over `shape_id`) used by
the generic `lift`. (Monomorphic sites never consult `shape_id`; it's purely for
the generic/residual path.)

**C. Thread the receiver type into the IR (Step 2's first task — see §7).**
`Rvalue::GetField(Atom, String)` and `Pattern::Record` must carry the resolved
record shape (closed field-name set) so emit can pick `struct.get k`. The type is
available at lowering: `ir/src/lower.rs` `ExprKind::FieldAccess` has
`receiver.ty`, and pattern sites have the subject type. Closed (`tail = None`) →
static `k`; open (`tail = Some`) → flows uniform.

**D. The polymorphism analysis.** Extend `ir/src/mono.rs` (which today does *not*
touch records) to specialize record-shape-polymorphic functions where every
relevant call site's shape is statically resolvable. Extend `ir/src/repr.rs` (or a
sibling pass) to assign each record-typed value a representation (nominal vs
uniform) and insert `lift`/`lower` coercions at boundaries — the same shape of
analysis as the existing Box/Unbox coercion insertion. This is the crux and the
main risk; it deserves its own validation milestone.

**E. Emit the monomorphic fast paths** (all in `wasm/src/emit.rs` /
`wasm/src/helpers/record.rs`):
- `MakeRecord` (closed shape) → `struct.new $shapeN` writing fields in sorted
  order; stamp `shape_id`.
- `GetField` (closed) → `ref.cast $shapeN` + `struct.get k`; CSE the cast across
  multiple reads of the same receiver.
- `RecordUpdate` (closed) → `struct.new $shapeN` copying base fields via
  `struct.get` and substituting overrides — *cleaner and faster than the current
  array-copy helper*.
- `Pattern::Record` (closed subject) → `ref.cast $shapeN` + `struct.get k` per
  bound field; this is also the natural place to finally implement
  `RecordRest::Bind` (which is currently unsupported on WASM — `emit.rs`): the
  bound rest is a record of statically-known shape at a *monomorphic* match site,
  so build its `$shapeRest` directly; at a *row-polymorphic* match site, it stays
  on the uniform path.
- Keep `__getfield`/`build_record_update_fn` and the uniform `MakeRecord` path as
  the fallback for open-shape sites.

**F. (Tier B, Step 3) Repr-type records concretely.** Carry `(ref $shapeN)` (not
`$value`) for record-typed locals/params/returns where the shape is statically
fixed, and specialize function signatures in `mono` so the cast in (E) disappears.
This is an extension of the existing Repr track and the function-type interner
(`FuncKind::Pluma(arity)` would need shape-aware variants).

---

## 7. Sequencing

We are **not** shipping the earlier "Step 1" (static slot resolution on the
existing array rep) as a separate milestone — most of its work is just (C), which
Step 2 needs anyway, and its unique part (an `array.get values[k]` leaf) would be
immediately replaced by `struct.get`. Instead:

**Step 2.0 — type-threading (first task, no representation change).** Implement
(C): thread the resolved record shape into `Rvalue::GetField` and `Pattern::Record`
and compute the sorted slot index. Land it *behind the existing scan* — i.e. still
call `__getfield`, but additionally assert (debug-only) that the resolved slot
matches the name the scan finds. Verify **zero snapshot diffs**. This isolates "is
the type at this site correct?" from the representation change that follows, so
later diffs have one cause, not three.

**Step 2.1 — the representation flip.** Implement A, B, D, E. Land the shape
registry, the nominal `MakeRecord`/`GetField`/`RecordUpdate`/pattern paths, the
repr-analysis + `lift`/`lower` coercions, and the monomorphization extension. The
generic consumers (`__eq`/`__tostring`/`wire`) and the schema machinery are
untouched — they run on the uniform rep reached via `lift`. Verify zero diffs over
the full fixture corpus (`tests/run`) and the `wire` fixtures. Implement
`RecordRest::Bind` on the monomorphic path as part of this (closing a current WASM
gap). Reaches **Tier A** (cast + `struct.get`).

**Step 3 — cast-free (Tier B).** Implement F: repr-type records concretely and
specialize signatures so the `ref.cast` drops out on statically-shaped paths.
Reaches the **true floor**. Independent and deferrable; do it only once Tier A is
measured and the cast is shown to matter.

---

## 8. Validation philosophy

VM-anchored, like the rest of the backend (`IR.md` "Validation philosophy"): the
bytecode VM is the oracle, and each step must produce **zero snapshot diffs** in
`tests/run` (and the analyzer snapshots where relevant). The WASM differential
harness compiles each fixture to `.wasm`, runs it in wasmtime, and diffs against
the VM's `status/stdout/stderr` — record-heavy fixtures
(`record-*`, `record-pattern-*`, the `wire` fixtures, `benchmarks/programs/record-access`)
are the targeted coverage. The debug-only slot/scan cross-check in Step 2.0 is an
extra in-vivo invariant on top of snapshots.

A microbench (cf. `tests/wasm_bench.rs`, the `reference_wasm_gc_boundary_bench`
memory) on `record-access` quantifies the win and tells us whether Step 3's
cast-removal is worth doing.

---

## 9. Open questions / risks

- **Residual polymorphism coverage.** How much real code can't be monomorphized
  and must pay `lift`/`lower`? The observed corpus is all direct-call with static
  shapes, but record-polymorphic *function values* (stored/passed/recursive) are
  expressible. Need a concrete enumeration during (D), and a `log`/diagnostic when
  a value is forced onto the uniform path so silent slow-paths don't hide.
- **`drop-name`-style producers.** Row-polymorphic functions that *return* a record
  of body-unknown shape are the sharpest case (uniform inside, closed outside).
  Confirm `mono` resolves them at call sites; the `lower` coercion is the safety
  net when it can't.
- **Code-size from monomorphization.** Per-shape specialization can multiply
  function bodies. Bound it (only specialize when call-site shapes are few and
  static); fall back to uniform past a threshold.
- **`shape_id` allocation & stability.** Ids are module-local and assigned by the
  scan pass; they're an internal contract between the struct stamp and the
  `lift`/`lower` dispatch, not observable. Keep deterministic (sorted name-set
  order) for reproducible output — note `wasm::emit` already has
  HashMap-iteration nondeterminism elsewhere (`project_wasm_helper_wat_dsl`
  memory), so build the registry order off sorted keys.
- **Per-field unboxing.** Fields are boxed `$value` for now. Unboxing scalar
  fields (an `int` field as inline `i64`) is a further win but a separate Repr
  extension; out of scope until Tier B lands.
- **Interaction with the schema / `wire`.** `wire` already derives field info from
  the schema value-tree, not the record's runtime names, so keeping it on the
  uniform rep (reached via `lift`) is sound and requires no `wire` change — verify
  this holds for decode, which *constructs* a uniform `$record` today.

---

## 10. Touchpoint index

| Concern | File / symbol |
|---|---|
| Value type hierarchy, `$record`, tags | `wasm/src/types.rs` (`T_VALUE`, `T_RECORD`, `TAG_RECORD`, `FuncTypes::encode`) |
| Record construct / read / update / pattern emit | `wasm/src/emit.rs` (`Rvalue::MakeRecord`, `Rvalue::GetField`, `Rvalue::RecordUpdate`, `Pattern::Record`) |
| Field-read & update helpers | `wasm/src/helpers/record.rs` (`build_getfield_fn`, `build_record_update_fn`) |
| Structural equality | `wasm/src/helpers/eq.rs` (`TAG_RECORD` arm) |
| `to-string` | `wasm/src/helpers/tostring.rs` (`TAG_RECORD` arm) |
| `wire` codec | `wasm/src/helpers/wire.rs` (`s_record` encode/decode arms) |
| Helper registry / DCE | `wasm/src/runtime.rs`, `wasm/src/helpers/mod.rs`, `wasm/src/scan.rs` |
| IR record nodes | `ir/src/types.rs` (`Rvalue::MakeRecord`, `Rvalue::GetField`, `Pattern::Record`) |
| Field-access lowering (receiver type available) | `ir/src/lower.rs` (`ExprKind::FieldAccess`) |
| Repr coercion model to mirror | `ir/src/repr.rs` (`Box`/`Unbox` insertion; record fields forced `Boxed`) |
| Monomorphization (to extend for shapes) | `ir/src/mono.rs` |
| Record type + open/closed tail | `compiler/src/types/type.rs` (`Type::Record(Vec<(String,Type)>, Option<usize>)`) |
| VM oracle | `codegen/src/from_ir.rs` (`Rvalue::GetField`), `vm/src/vm.rs` (`Instruction::GetField`), `vm/src/value.rs` (`Value::Record`) |
| Row-poly test fixtures | `tests/run/record-pattern-row-poly`, `record-pattern-named-rest`, `benchmarks/programs/record-access` |
