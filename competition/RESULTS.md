# Pluma vs Python, Ruby, and Node.js — benchmark results

_Best of 5 runs, wall-clock seconds (lower is better). Generated 2026-06-01 21:23:08 PDT._

Correctness: every implementation agreed on every output.

## Environment

| component | version |
|---|---|
| host | `Darwin arm64` |
| pluma | git `db8273d` (release build) |
| python3 | Python 3.11.2 |
| ruby | ruby 2.6.10p210 |
| node | v25.2.1 |

## Results

| benchmark | exercises | pluma-vm | pluma-wasm | python3 | ruby | node | vm vs best | wasm vs best | output |
|---|---|--:|--:|--:|--:|--:|--:|--:|:--:|
| `fib` | naive recursion | 0.55 | 0.10 | 0.27 | 0.30 | 0.05 | 11.0x | 2.0x | ok |
| `mandelbrot` | float64 escape loop | 1.14 | 0.17 | 0.39 | 0.71 | 0.05 | 22.8x | 3.4x | ok |
| `primes` | integer trial division | 1.34 | 0.38 | 0.50 | 0.58 | 0.05 | 26.8x | 7.6x | ok |
| `sort` | sort + checksum | 1.40 | 0.31 | 0.04 | 0.08 | 0.06 | 35.0x | 7.8x | ok |
| `dict` | hash-map tally | 0.41 | 0.17 | 0.06 | 0.09 | 0.04 | 10.2x | 4.2x | ok |
| `string` | join / split / upcase | 1.16 | 0.25 | 0.03 | 0.07 | 0.04 | 38.7x | 8.3x | ok |
| `tree` | build + fold a tree | 1.23 | 0.51 | 3.36 | 0.98 | 0.12 | 10.2x | 4.2x | ok |
| `collections` | map / filter / fold | 0.65 | 0.22 | 0.12 | 0.16 | 0.14 | 5.4x | 1.8x | ok |

One-time cost to compile all 8 benchmarks to WasmGC artifacts: **0.04s** total (not included in the `pluma-wasm` per-run times).

## How to read this

- Pluma ships **two backends over one IR**, so it appears twice:
  - `pluma-vm` — `pluma run <src>`, the reference VM interpreter. The time
    includes front-end compilation, because that is what the dev loop costs
    every run.
  - `pluma-wasm` — the WasmGC deploy artifact (`pluma build` once, then
    `pluma run <out>.wasm` in the embedded wasmtime host). Since you build
    once and run many, the per-run time measures *executing* the artifact;
    the one-time build cost is reported separately above.
- `vm vs best` / `wasm vs best` divide each Pluma backend's time by the fastest
  competitor's time (greater than 1× means Pluma is slower; less than 1× means
  faster).
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
