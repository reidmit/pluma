# Analyzer performance investigation

Tracking the cost of the compiler frontend (type analysis), using the stdlib
test suite as the workload. This doc is a living log: record each run's numbers
so we can compare across changes (and across context resets).

**Workload:** `pluma test compiler/src/stdlib` — 14 `*.test.pa` modules, 307
test cases. It exercises the analyzer hard (lots of polymorphic stdlib calls)
while the VM/codegen portion is trivial, so it's effectively a type-checker
benchmark.

## How to reproduce

```sh
just build-release

# Coarse phase breakdown (discover / check / codegen / vm) to stderr:
PLUMA_TIMING=1 ./target/release/cli test compiler/src/stdlib

# Per-module analyzer phase breakdown (constrain / unify / try / discharge / annotate):
PLUMA_TIMING=2 ./target/release/cli test compiler/src/stdlib

# Wall-clock for the whole process (includes startup):
time ./target/release/cli test compiler/src/stdlib
```

`PLUMA_TIMING` instrumentation lives in `cli/src/main.rs` (coarse phases) and
`compiler/src/{compiler.rs,analyzer.rs}` (per-module parse/analyze + per-phase).
It's gated behind the env var and ~free when unset, so it's committed as a
permanent profiling tool.

Run a few times and take the steady-state (the first run is cold). Numbers below
are from an M-series mac, release build.

## Findings (root cause)

1. **The frontend is ~99.7% of the time.** Test execution of all 307 cases is
   ~0.4ms; codegen ~1.6ms; parsing ~10ms total. Everything else is type
   analysis. This is a type-checker benchmark, not a VM one.

2. **`unify` is the entire analysis cost** and it's **O(n²)** in the number of
   constraints per module. Every other phase (constrain, discharge, annotate)
   is sub-millisecond per module.

3. **Why O(n²):** `unify_eq_constraints` (`analyzer.rs:~3363`) is textbook
   substitution-passing Algorithm W:
   ```rust
   let subst_first = self.unify_eq_constraint(&constraints[0]);
   let rest = subst_first.apply_to_constraints(&constraints[1..]); // rewrites WHOLE tail
   let subst_rest = self.unify(&rest);
   subst_first.compose(subst_rest)                                 // re-clones growing map
   ```
   Each constraint triggers an `apply_to_constraints` over all remaining
   constraints; each `apply_to_type` (`substitution.rs:~47`) deep-clones a
   `Type` tree (no `Rc`/interning). Σ O(n) = O(n²), with a large constant from
   by-value `Type` cloning. Measured cost is a steady **~65ns per Type-node
   clone**; the node-clone *count* is what explodes (list.test: 2.1M clones for
   616 constraints).

4. **Secondary:** the `try`-dispatch fixpoint loop (`analyzer.rs:~575`) re-runs
   full `unify` over the *entire* accumulated constraint set each iteration —
   so `option.test`/`result.test` pay ~20ms in `try` on top of `unify`.

5. **Cost is per-module quadratic** (Σnᵢ² ≪ (Σnᵢ)²), so splitting big test
   modules is a cheap near-linear win; merging them all would be catastrophic.

## Fix plan

- [x] **#1 Union-find / mutable substitution (the real fix).** DONE — see the
  2026-05-26 "after #1" run below. `unify_eq_constraints` is now a worklist over
  a chained `bindings`/`rows` map resolved lazily (`resolve_head`), normalized to
  an idempotent `Substitution` once at the end (`deep_resolve`). Error sites
  deep-resolve their types so messages are unchanged. ~30× wall-clock, ~35×
  `check`, ~245× on the worst module's `unify`. Zero snapshot changes.
- [ ] **#2 Kill the eager rewrite (cheaper, keeps structure).** Maintain one
  accumulating substitution, shallow-resolve vars on demand instead of
  `apply_to_constraints` over the whole tail each step.
- [ ] **#3 `Rc<Type>` / interning.** Attack the ~65ns per-clone constant rather
  than the n².
- [ ] **#4 `try` loop.** Re-unify only newly added constraints, or memoize,
  instead of re-solving the full set each round.

## Runs log

Append a row per measured change. `total` = `time` real, steady-state. Per-module
unify times in ms from `PLUMA_TIMING=2`.

### Baseline — commit `a3e785c` (branch `rename-map-to-dict`), 2026-05-26

Coarse (`PLUMA_TIMING=1`): discover+setup 0.1 · **check ~588** · codegen 1.6 ·
vm register 0.4 · vm run 0.4 (307 cases) · **total in-proc ~590ms**.
`time` real: ~0.59s steady (0.77s cold).

Per-module (`PLUMA_TIMING=2`), sorted by constraint count:

| module       | constraints | constrain | unify  | try   | annotate |
|--------------|------------:|----------:|-------:|------:|---------:|
| assert.test  |          52 |      0.04 |   1.62 |  0.00 |     0.01 |
| hex.test     |         119 |      0.09 |   7.15 |  0.01 |     0.02 |
| ref.test     |         132 |      0.07 |   5.40 |  0.01 |     0.01 |
| base64.test  |         169 |      0.11 |  16.22 |  0.02 |     0.03 |
| random.test  |         180 |      0.11 |  12.66 |  0.02 |     0.03 |
| uuid.test    |         200 |      0.13 |  18.29 |  0.02 |     0.03 |
| regex.test   |         217 |      0.17 |  23.16 |  0.02 |     0.03 |
| option.test  |         226 |      0.13 |  17.75 | 18.14 |     0.03 |
| result.test  |         237 |      0.12 |  19.62 | 20.00 |     0.03 |
| math.test    |         250 |      0.22 |  26.05 |  0.03 |     0.03 |
| bytes.test   |         332 |      0.26 |  62.30 |  0.04 |     0.06 |
| string.test  |         416 |      0.30 |  93.31 |  0.05 |     0.07 |
| json.test    |         434 |      0.42 |  83.57 |  0.05 |     0.07 |
| list.test    |         616 |      0.30 | 142.19 |  0.07 |     0.09 |

apply_to_type node-visits (O(n²) evidence): hex 116K, math 409K, string 1.42M,
list 2.10M — ~65ns/visit, steady.

### After #1 union-find — branch `rename-map-to-dict`, 2026-05-26

Worklist unifier (chained `bindings`/`rows`, lazy `resolve_head`, single
`deep_resolve` normalization). All 275 workspace tests + 307 stdlib cases pass,
**zero snapshot changes**.

Coarse (`PLUMA_TIMING=1`): discover+setup 0.1 · **check 16.9** · codegen 1.4 ·
vm register 0.4 · vm run 0.4 · **total in-proc ~20ms**.
`time` real: **~0.02s steady** (0.19s cold) — was ~0.59s.

Per-module unify (`PLUMA_TIMING=2`), vs baseline:

| module       | constraints | unify before | unify after | speedup |
|--------------|------------:|-------------:|------------:|--------:|
| hex.test     |         119 |         7.15 |        0.14 |    ~51× |
| math.test    |         250 |        26.05 |        0.22 |   ~118× |
| bytes.test   |         332 |        62.30 |        0.42 |   ~148× |
| string.test  |         416 |        93.31 |        0.54 |   ~173× |
| json.test    |         434 |        83.57 |        0.47 |   ~178× |
| list.test    |         616 |       142.19 |        0.58 |   ~245× |

`try` loop (re-runs unify each fixpoint round): option.test 18.1→0.34,
result.test 20.0→0.42. `unify` is now linear-ish in constraint count and no
longer the bottleneck — it's on par with constrain/annotate. Remaining `check`
time is spread evenly across parse/constrain/unify/annotate.

**Summary: ~588ms → ~17ms `check` (~35×); ~0.59s → ~0.02s wall (~30×).**
