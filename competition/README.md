# Competition: Pluma vs Grain, Python, Ruby, Node, Bun, and Deno

A small, honest cross-language benchmark suite. Each subfolder holds the **same
program written once per language**, idiomatically. Every implementation prints
byte-identical output, so the runner can confirm the programs actually computed
the same thing before it trusts the timings.

[**Grain**](https://grain-lang.org) is the most interesting comparison here: like
Pluma, it is a small statically-typed functional language that **compiles to
WebAssembly** — so this is two wasm-targeting functional languages run
side-by-side, not just Pluma against interpreters and JITs. But they take opposite
wasm strategies, which is the single biggest reason their numbers diverge:

- **Pluma** emits **WasmGC** — the WebAssembly GC proposal's heap types (`struct`,
  `array`, `ref`) — so its boxes are first-class to V8's optimizer and its
  allocation/collection are V8's own generational GC. Small ints ride as i31
  tagged values (no box at all).
- **Grain** emits **plain WebAssembly over linear memory**, with its own
  **reference-counting** runtime managing the heap by hand (its compiled `.wat`
  has no GC heap types at all — it's all `i32.load`/`i32.store` plus incRef/decRef),
  and a universal **boxed, arbitrary-precision `Number`** whose `+`/`*`/`%` lower
  to runtime dispatch calls rather than native wasm arithmetic.

Crucially, **both run their wasm under the same V8** — `grain run` bundles Node,
and Grain's crash traces are V8's (`wasm://wasm/…:wasm-function[N]`,
`RangeError: Maximum call stack size exceeded`). So the large gap between
`pluma-v8` and `grain-wasm` is *not* JIT-vs-no-JIT; it is what each compiler hands
V8. Pluma hands it native primitives (i31 ints, WasmGC structs) that TurboFan and
the generational GC optimize directly; Grain hands it a hand-rolled managed
runtime — generic boxed-`Number` arithmetic routed through dispatch functions, and
manual refcount traffic on every value — that V8 must execute literally because it
cannot prove any of it away. (More in *Reading the results* below.)

## Running

```sh
cargo build --release --bin pluma   # build the Pluma CLI first
competition/run.sh                # best of 5 runs (pass a number to change)
competition/run.sh 10             # best of 10
```

`run.sh` invokes each program the way you would from a shell —

```
./target/release/pluma run <prog>.wasm   ./target/release/pluma run <prog>.pa
grain run <prog>.gr.wasm   grain <prog>.gr
python3 <prog>.py   ruby <prog>.rb   node <prog>.js   bun <prog>.js   deno run <prog>.js
```

(The Grain columns are skipped — reading `n/a` — if `grain` is not on your
`PATH`. Install it from [grain-lang.org](https://grain-lang.org); the suite was
built against Grain 0.7.)

— and reports the best wall-clock time over N runs. **Times include process
startup.** The `pluma-v8` column is *built once* up front (`pluma build`), so its
per-run time is execution only; that one-time build cost is summed and reported
separately, not folded into the per-run numbers. The `pluma-src` column instead
runs straight from source every time, so it folds the full compile pipeline into
each run — the gap between the two is what compiling costs.

## The languages

| column | engine | shape |
|---|---|---|
| `pluma-v8`  | WasmGC artifact under **V8** | compiled to wasm once (`pluma build`), then `pluma run <out>.wasm` — *run what you ship* (execution only) |
| `pluma-src` | WasmGC under **V8**, from source | `pluma run <prog>.pa` — compiles *and* runs each invocation, so the time includes the full parse/analyze/compile pipeline |
| `grain-wasm`| linear-memory wasm + refcounting runtime, **under V8** (bundled Node) | compiled to wasm once (`grain compile --release`), then `grain run <out>.gr.wasm` — Grain's *run what you ship*, execution only |
| `grain-src` | linear-memory wasm + refcounting runtime, **under V8**, from source | `grain <prog>.gr` — compiles *and* runs each invocation, so the time includes the Grain compiler pipeline |
| `python3`   | CPython | bytecode interpreter |
| `ruby`      | CRuby (MRI) | bytecode interpreter |
| `node`      | V8 | JIT |
| `bun`       | JavaScriptCore | JIT — the one *cross-engine* JS data point (a different JIT from V8) |
| `deno`      | V8 | JIT — the same engine as node, runs the same `.js` |

`grain-wasm` and `grain-src` are the exact analogs of `pluma-v8` and `pluma-src`:
the first is the prebuilt wasm artifact you'd deploy (execution only, build cost
summed separately), the second compiles from source on every invocation. Because
both Pluma and Grain compile ahead of time to WebAssembly, the `pluma-v8` ↔
`grain-wasm` pair is the cleanest like-for-like in the suite — same broad target
(wasm), but a different memory model (WasmGC vs linear-memory refcounting) and a
different execution engine (V8 vs Grain's runner) underneath.

Pluma is measured as **`pluma-v8`**: the WasmGC deploy artifact, executed under
**V8** (the engine `pluma run` uses, so this is *run what you deploy*). V8's
generational GC is what makes Pluma's boxed-value IR fast — it bulk-frees the
per-iteration transients a reference-counting collector would churn on. Pluma's
own output is the reference every other column is diffed against.

**`pluma-src`** is the same artifact run straight from source (`pluma run
<prog>.pa`), so each timing includes the whole front-end — tokenize, parse,
HM-style analyze, IR lowering, WasmGC codegen — *and* execution, in one process.
The gap between `pluma-src` and `pluma-v8` is therefore the per-invocation compile
cost, and across the suite it is small (typically **~10–30 ms**): the pipeline is
cheap enough that `pluma run foo.pa` feels interpreted while still being compiled.
`vs best` still compares the deploy artifact (`pluma-v8`) to the fastest other
language.

`node`, `bun`, and `deno` all run the **same `.js` file**. A benchmark a language
doesn't implement reads `n/a`.

Each timed command runs under a wall-clock cap (default 30 s, override with
`RUN_TIMEOUT`) so a workload one language handles far more slowly than the rest
can't wedge the whole suite — such a cell reads `>30s` rather than hanging.

## The benchmarks

| # | folder           | exercises                                          |
|---|------------------|----------------------------------------------------|
| 1 | `01-fib`         | naive recursion / function-call overhead           |
| 2 | `02-mandelbrot`  | float64 arithmetic in a tight escape loop          |
| 3 | `03-primes`      | integer arithmetic + modulo (trial division)       |
| 4 | `04-sort`        | sorting a large list, then an order-sensitive fold |
| 5 | `05-dict`        | hash-map insert/lookup (word-frequency tally)      |
| 6 | `06-string`      | text throughput: build / join / split / upcase     |
| 7 | `07-tree`        | building and folding a recursive data structure    |
| 8 | `08-collections` | functional `map` / `filter` / `fold` pipeline      |
| 9 | `09-interp`      | AST interpreter — enums + exhaustive pattern match  |
| 10| `10-nbody`       | float64 over record structs + per-step allocation  |
| 11| `11-sieve`       | mutable-array marking in a tight loop (`list.set`)  |
| 12| `12-json`        | JSON parse / aggregate / re-serialize round-trip    |
| 13| `13-regex`       | regex scan + capture over a large generated text    |

### How the Grain ports differ

The Grain programs compute byte-identical output, but a few of them are shaped by
Grain-specific facts worth calling out (each `.gr` file documents its own):

- **Floats use Grain's `Float64`, not the default `Number`.** Grain's `Number` is
  arbitrary-precision — `10 / 3` is the *rational* `10/3`, not a double — so
  `mandelbrot` and `nbody` use the fixed-width `Float64` type explicitly to get
  the same IEEE-754 results every other language produces. (Its operators are
  bound to named helpers so the integer loop counters keep `Number`'s operators.)
- **`collections` uses Grain's `Array`, not its `List`.** Grain's `List.map` /
  `List.filter` are cons-list operations that recurse one stack frame per element
  and overflow past ~100k, so the 1M-element pipeline runs on Grain's contiguous
  `Array` — the same shape Pluma's array-backed `list` and the other languages'
  arrays already use here.
- **`mandelbrot` and `sieve` use loops.** Grain has real `while` loops and mutable
  arrays, so these take the imperative mark/scan shape (the same one Python and JS
  run) rather than the tail recursion Pluma falls back to for lack of loops.
- **`regex` runs per line, then de-overlaps.** Grain's regex engine is pure Grain
  (no native library), and two of its properties shape the port: it recurses per
  character, so `findAll` over the whole multi-line text overflows the stack — the
  scan runs per line instead; and `findAll` reports a match at *every* start offset
  (overlapping), so a greedy `start >= lastEnd` pass reproduces the leftmost-longest
  non-overlapping match set every other engine reports.

To keep integer outputs byte-identical across languages, every benchmark keeps
its arithmetic inside float64's exact-integer range (`< 2^53`), usually with a
`mod` — so even Node, whose numbers are doubles, agrees with the bignum and
i64 languages. The float-heavy rows (`mandelbrot`, `nbody`) emit an integer
derived from the result (a count, or the energy scaled and floored), since IEEE
`+ - * / sqrt` are identical everywhere but decimal *formatting* is not.

## Reading the results, fairly

This compares **idiomatic code in each language**, not equivalent machine work:

- **Compiling helps a lot, and a JIT helps more.** The deploy artifact under V8
  lands at or near parity with Node on the tightest compute rows (`fib`,
  `mandelbrot`, `primes`, `interp`, `collections`) — the same V8, the same kind of
  generational GC.
- **Compiling to wasm is not enough on its own — what you hand V8 decides.**
  Grain compiles ahead of time to wasm just like Pluma, and `grain run` executes it
  under the *same V8* `pluma-v8` uses (its runner bundles Node; its crash traces are
  V8's). Yet `grain-wasm` trails every other column here, usually by 1–2 orders of
  magnitude, and it trails on the *pure-compute* rows (`fib`, `primes`,
  `mandelbrot`) too — so it is neither a JIT story nor only a GC story. It is the
  *code each compiler emits*. Three things compound: (1) Grain's default `Number` is
  a universal boxed, arbitrary-precision type, so `+`/`*`/`%` lower to calls into
  runtime dispatch routines (`numberTimes`, `isSimpleNumber` show up right in the
  traces) instead of native `i64`/`f64` ops — this alone sinks the integer rows;
  (2) its reference-counting runtime adjusts a refcount on every boxed value bound,
  passed, or returned (`incRef`/`decRef` everywhere), explicit memory traffic V8
  cannot elide; and (3) it heap-boxes values (including `Float64`) in linear memory
  via a hand-written allocator, where Pluma's boxes are WasmGC structs V8
  bump-allocates and its generational GC bulk-frees. The gap widens exactly where
  allocation is heaviest (`nbody`, `tree`, `sort`), but the compute-only rows
  isolate the boxed-arithmetic-plus-refcount cost by itself. By contrast Pluma hands
  V8 native primitives — i31-tagged ints (no box) and WasmGC structs first-class to
  TurboFan. This is a design-point difference, not a defect: Grain buys exact
  rationals and arbitrary precision (`10 / 3` really is `10/3`) and emits small,
  portable, self-contained modules; these tiny microbenchmarks also pay its runner's
  fixed per-invocation startup on every row. Worth noting separately: Grain's
  `--release` build (binaryen/`wasm-opt`) is *slow* — tens of seconds for some of
  these programs — so its one-time compile-to-wasm cost is far above Pluma's.
- **`interp` is where the functional core shows.** An AST of nominal enum nodes,
  evaluated under exhaustive `when` matching, is exactly what Pluma is built for —
  it beats both interpreters (CPython, CRuby) by a wide margin and edges ahead of
  Node.
- **`nbody` is where Pluma pays.** Records are immutable, so each simulation step
  rebuilds every body — millions of short-lived boxed structs. That is heavy
  allocation the scalar-mutating languages avoid entirely; it is Pluma's worst
  row, and an honest one. (The enduring lever is value representation —
  `ir::repr` unboxing — not the collector.)
- **`sieve` flips the interpreters' weakness into view.** A tight mark/scan loop
  with array mutation is what CPython and CRuby are slowest at; the WasmGC
  artifact under V8, and the JITs, are far ahead of them here.
- **`sort`, `string`, `json`, and `regex`** run Pluma-level stdlib code —
  `list.sort` is a merge sort written in Pluma calling a comparison closure per
  compare; the string ops and the entire JSON codec are Pluma too — against the
  other languages' *C-level* sort, string, and JSON routines. That is a deliberate
  idiomatic-vs-idiomatic comparison, and it is where Pluma pays the most.
- **`regex` is a pure-Pluma engine against C libraries.** Pluma's regex engine is
  a pure-Pluma recursive backtracker (a backtick pattern reifies to a
  `regex-pattern` tree the matcher walks) — no native engine, no host call, run
  under V8 like everything else. Every competitor here calls a *C* regex library
  (CPython's `re`, CRuby's Onigmo, V8's Irregexp), several with literal-prefix and
  `memchr`-style scan optimizations a tree-walking backtracker has none of, so a
  gap is expected. (The matching itself is cheap; `find-all`'s cost is building
  one `match` record per hit — keep that in mind before scanning for millions of
  matches in a hot loop.)
- **`dict`** is a mutable open-addressing hash table (`insert`/`remove` mutate in
  place and return the map), so the word-frequency tally is O(1) per key, in line
  with the other languages' maps.
- **Small inputs are startup-dominated.** Where a competitor finishes in ~0.02–
  0.06 s it is essentially measuring interpreter startup, not the workload. `bun`
  starts fast and so flatters these rows.
- **Pluma is not always behind** — see `tree` and `interp`, where building and
  walking millions of nominal nodes beats CPython's object allocation outright.

Inputs are sized so Pluma lands in a ~1–2 s window per benchmark; adjust the
sizes in each program (kept in sync across all the language files) to taste.

(Earlier revisions ran the artifact under wasmtime's `null` and `drc` collectors,
and against a bytecode-VM column; both have since been retired — Pluma has a
single WasmGC backend, run under V8, the engine you deploy.)
