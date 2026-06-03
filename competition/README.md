# Competition: Pluma vs Python, Ruby, and Node.js

A small, honest cross-language benchmark suite. Each subfolder holds the **same
program written four times** — once in Pluma (`.pa`) and once each in idiomatic
Python, Ruby, and Node.js. Every implementation prints byte-identical output, so
the runner can confirm the four programs actually computed the same thing before
it trusts the timings.

## Running

```sh
cargo build --release --bin cli   # build the Pluma CLI first
competition/run.sh                # best of 5 runs (pass a number to change)
competition/run.sh 10             # best of 10
```

`run.sh` invokes each program the way you would from a shell —

```
./target/release/cli <prog>     python3 <prog>.py
ruby <prog>.rb                  node <prog>.js
```

— and reports the best wall-clock time over N runs. **Times include process
startup.** The `pluma-v8` deploy artifact is built once up front, so its per-run
time is execution only (the build cost is reported separately).

Pluma is measured as **`pluma-v8`** — the WasmGC deploy artifact: `pluma build
--target server` *once*, then `pluma run <out>.wasm`, executed under **V8** (the
`pluma run` engine, so this is *run what you ship*). Because you build once and run
many, the per-run time measures *executing* the artifact; the one-time compile-to-
wasm cost is summed and reported separately, not folded into the per-run number.
V8's generational GC is what makes Pluma's boxed-value IR fast here. Pluma's own
output is the reference the other languages are diffed against.

(Earlier revisions ran the artifact under wasmtime's `null` and `drc` collectors,
and against a bytecode-VM column; both have since been retired — Pluma has a single
WasmGC backend, run under V8, the engine you deploy.)

Each timed command runs under a wall-clock cap (default 30 s, override with
`RUN_TIMEOUT`) so a workload one backend handles far more slowly than the rest
can't wedge the whole suite — such a cell reads `>30s` rather than hanging.

## The benchmarks

| # | folder           | exercises                                         |
|---|------------------|---------------------------------------------------|
| 1 | `01-fib`         | naive recursion / function-call overhead          |
| 2 | `02-mandelbrot`  | float64 arithmetic in a tight escape loop         |
| 3 | `03-primes`      | integer arithmetic + modulo (trial division)      |
| 4 | `04-sort`        | sorting a large list, then an order-sensitive fold |
| 5 | `05-dict`        | hash-map insert/lookup (word-frequency tally)     |
| 6 | `06-string`      | text throughput: build / join / split / upcase    |
| 7 | `07-tree`        | building and folding a recursive data structure   |
| 8 | `08-collections` | functional `map` / `filter` / `fold` pipeline     |

## Reading the results, fairly

This compares **idiomatic code in each language**, not equivalent machine work:

- **Compiling helps a lot.** Across the compute-heavy rows (`fib`, `mandelbrot`,
  `primes`, `tree`, `sort`, `dict`) the WasmGC artifact under V8 runs ~10–30×
  faster than the VM, and unlike the VM its time excludes front-end compilation.
- **Node** runs on a JIT (V8); CPython and CRuby are interpreters, as is Pluma's
  reference VM. But `pluma-v8` deploys the *same* WasmGC artifact onto the *same*
  V8 — so on the tightest compute rows (`fib`, `mandelbrot`, `primes`,
  `collections`) it lands at parity with or just ahead of Node, rather than losing
  to it by a wide margin.
- **`sort` and `string`** run Pluma-level stdlib code — `list.sort` is a merge
  sort written in Pluma calling a comparison closure per compare; the string ops
  are Pluma too — against the other languages' *C-level* sort and string
  routines. That is a deliberate idiomatic-vs-idiomatic comparison, and it is
  where Pluma pays the most.
- **`dict`** used to be the worst row (an immutable map that deep-copied on every
  insert → O(n²)). Both backends now use a persistent, structurally-shared map so
  `dict.insert` is O(log n), still immutable, still insertion-ordered: the VM is
  backed by `im_rc`, and WasmGC by a hand-written hash-array-mapped trie keyed on a
  structural hash (path-copied nodes shared by reference; see
  `wasm/src/helpers/dict.rs`). The 200k-insert benchmark that once never finished
  on WasmGC now runs faster there than on the VM.
- **Small inputs are startup-dominated.** Where a competitor finishes in ~0.02–
  0.06 s it is essentially measuring interpreter startup, not the workload.
- **Pluma is not always last** — see `tree`, where building millions of nominal
  enum nodes beats CPython's object allocation.

Inputs are sized so Pluma lands in a ~1–2 s window per benchmark; adjust the
sizes in each program (kept in sync across all four files) to taste.
