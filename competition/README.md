# Competition: Pluma vs Python, Ruby, Node, Bun, Deno, LuaJIT, and Haskell

A small, honest cross-language benchmark suite. Each subfolder holds the **same
program written once per language**, idiomatically. Every implementation prints
byte-identical output, so the runner can confirm the programs actually computed
the same thing before it trusts the timings.

## Running

```sh
cargo build --release --bin cli   # build the Pluma CLI first
competition/run.sh                # best of 5 runs (pass a number to change)
competition/run.sh 10             # best of 10
```

`run.sh` invokes each program the way you would from a shell —

```
./target/release/cli run <prog>.wasm   python3 <prog>.py    ruby <prog>.rb
node <prog>.js   bun <prog>.js   deno run <prog>.js   luajit <prog>.lua
./<prog>                                # the ghc -O2 binary
```

— and reports the best wall-clock time over N runs. **Times include process
startup.** The two compiled columns — `pluma-v8` and `haskell` — are *built once*
up front, so their per-run time is execution only; those one-time build costs are
summed and reported separately, not folded into the per-run numbers.

## The languages

| column | engine | shape |
|---|---|---|
| `pluma-v8`  | WasmGC artifact under **V8** | compiled to wasm once (`pluma build`), then `pluma run <out>.wasm` — *run what you ship* |
| `python3`   | CPython | bytecode interpreter |
| `ruby`      | CRuby (MRI) | bytecode interpreter |
| `node`      | V8 | JIT |
| `bun`       | JavaScriptCore | JIT — the one *cross-engine* JS data point (a different JIT from V8) |
| `deno`      | V8 | JIT — the same engine as node, runs the same `.js` |
| `luajit`    | LuaJIT | tracing JIT — the speed ceiling on the compute rows |
| `haskell`   | GHC native code | compiled, lazy-functional — Pluma's nearest paradigm cousin |

Pluma is measured as **`pluma-v8`**: the WasmGC deploy artifact, executed under
**V8** (the engine `pluma run` uses, so this is *run what you deploy*). V8's
generational GC is what makes Pluma's boxed-value IR fast — it bulk-frees the
per-iteration transients a reference-counting collector would churn on. Pluma's
own output is the reference every other column is diffed against.

`node`, `bun`, and `deno` all run the **same `.js` file**; `luajit` runs a `.lua`
port; `haskell` a `.hs` compiled with `ghc -O2`. Not every language implements
every benchmark — `luajit` and `haskell` skip `json`, because neither ships a JSON
codec in its standard library and an idiomatic port would mean vendoring a
third-party one. Such a cell reads `n/a`.

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
  generational GC. `luajit`'s tracing JIT is usually the ceiling on those rows.
- **`interp` is where the functional core shows.** An AST of nominal enum nodes,
  evaluated under exhaustive `when` matching, is exactly what Pluma is built for —
  it beats both interpreters (CPython, CRuby) by a wide margin and edges ahead of
  Node, with only the *compiled* columns (Haskell, LuaJIT) further ahead.
- **`nbody` is where Pluma pays.** Records are immutable, so each simulation step
  rebuilds every body — millions of short-lived boxed structs. That is heavy
  allocation the scalar-mutating languages avoid entirely; it is Pluma's worst
  row, and an honest one. (The enduring lever is value representation —
  `ir::repr` unboxing — not the collector.)
- **`sieve` flips the interpreters' weakness into view.** A tight mark/scan loop
  with array mutation is what CPython and CRuby are slowest at; the WasmGC
  artifact under V8, and the JITs, are far ahead of them here.
- **`sort`, `string`, and `json`** run Pluma-level stdlib code — `list.sort` is a
  merge sort written in Pluma calling a comparison closure per compare; the string
  ops and the entire JSON codec are Pluma too — against the other languages'
  *C-level* sort, string, and JSON routines. That is a deliberate
  idiomatic-vs-idiomatic comparison, and it is where Pluma pays the most.
- **`dict`** is a mutable open-addressing hash table (`insert`/`remove` mutate in
  place and return the map), so the word-frequency tally is O(1) per key, in line
  with the other languages' maps.
- **Small inputs are startup-dominated.** Where a competitor finishes in ~0.02–
  0.06 s it is essentially measuring interpreter (or RTS) startup, not the
  workload. `bun` and the compiled columns start fast and so flatter these rows.
- **Pluma is not always behind** — see `tree` and `interp`, where building and
  walking millions of nominal nodes beats CPython's object allocation outright.

Inputs are sized so Pluma lands in a ~1–2 s window per benchmark; adjust the
sizes in each program (kept in sync across all the language files) to taste.

(Earlier revisions ran the artifact under wasmtime's `null` and `drc` collectors,
and against a bytecode-VM column; both have since been retired — Pluma has a
single WasmGC backend, run under V8, the engine you deploy.)
