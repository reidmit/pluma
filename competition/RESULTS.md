# Pluma vs Python, Ruby, and Node.js — benchmark results

_Best of 5 runs, wall-clock seconds (lower is better). Generated 2026-06-02 12:58:27 PDT._

Correctness: every implementation agreed on every output.

## Environment

| component | version |
|---|---|
| host | `Darwin arm64` |
| pluma | git `addde8f` (release build) |
| python3 | Python 3.11.2 |
| ruby | ruby 2.6.10p210 |
| node | v25.2.1 |

## Results

| benchmark | exercises | pluma-vm | pluma-v8 | python3 | ruby | node | vm vs best | v8 vs best | output |
|---|---|--:|--:|--:|--:|--:|--:|--:|:--:|
| `fib` | naive recursion | 0.54 | 0.03 | 0.27 | 0.30 | 0.05 | 10.8x | 0.6x | ok |
| `mandelbrot` | float64 escape loop | 1.13 | 0.04 | 0.39 | 0.70 | 0.05 | 22.6x | 0.8x | ok |
| `primes` | integer trial division | 1.33 | 0.04 | 0.49 | 0.58 | 0.05 | 26.6x | 0.8x | ok |
| `sort` | sort + checksum | 1.40 | 0.07 | 0.03 | 0.07 | 0.06 | 46.7x | 2.3x | ok |
| `dict` | hash-map tally | 0.41 | 0.14 | 0.06 | 0.09 | 0.04 | 10.2x | 3.5x | ok |
| `string` | join / split / upcase | 0.75 | 0.04 | 0.02 | 0.07 | 0.03 | 37.5x | 2.0x | ok |
| `tree` | build + fold a tree | 1.20 | 0.51 | 3.27 | 0.96 | 0.11 | 10.9x | 4.6x | ok |
| `collections` | map / filter / fold | 0.64 | 0.11 | 0.12 | 0.16 | 0.14 | 5.3x | 0.9x | ok |

One-time cost to compile all 8 benchmarks to WasmGC artifacts: **0.07s** total (not included in the per-run `pluma-v8` times).

## How to read this

- Pluma ships **two backends over one IR**, so it appears twice:
  - `pluma-vm` — `pluma run --vm <src>`, the reference bytecode interpreter.
    It is the dev/test oracle (and the differential reference the deploy backend
    is cross-checked against), **not** a deploy target. The time includes front-end
    compilation, because that is what the dev loop costs every run.
  - `pluma-v8` — the WasmGC artifact you deploy (`pluma build` once, then
    `pluma run <out>.wasm`), executed under **V8** — the default `pluma run`
    engine, so this is *run what you ship*. The per-run time measures executing
    the artifact; the one-time compile-to-wasm cost is reported separately above.
    V8's **generational garbage collector** is what makes Pluma's boxed-value IR
    fast here: it bulk-frees the short-lived per-iteration allocations that a
    reference-counting collector would churn on one at a time.
- `vm vs best` / `v8 vs best` divide a Pluma time by the fastest competitor's
  time (greater than 1× means Pluma is slower; less than 1× means faster).
  `v8 vs best` is the deploy reality — the artifact you ship vs the field.
- `output` = `ok` means Pluma (both backends) and all three competitors printed
  byte-identical results; `MISMATCH` means they disagreed and the row should not
  be trusted.
- A time cell may instead read `n/a` (tool not installed), `ERR` (exited non-zero),
  or `>30s` (still running when the per-run cap fired — the workload is
  far slower on that backend, not crashed). Such cells are excluded from the
  ratio and the output check.
- This compares **idiomatic code in each language**. `core.dict` is a persistent,
  structurally-shared map (O(log n) insert, immutable, insertion-ordered);
  `list.sort` is a Pluma-level merge sort and the string ops are Pluma-level too,
  versus the other languages' native mutable maps and C-level sort/string routines.
- Where a competitor finishes in well under ~0.1 s it is essentially measuring
  interpreter startup, not the workload.
- Regenerate with `competition/run.sh [RUNS]`.
