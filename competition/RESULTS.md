# Pluma vs Python, Ruby, and Node.js — benchmark results

_Best of 5 runs, wall-clock seconds (lower is better). Generated 2026-06-02 08:33:21 PDT._

Correctness: every implementation agreed on every output.

## Environment

| component | version |
|---|---|
| host | `Darwin arm64` |
| pluma | git `30fb233` (release build) |
| python3 | Python 3.11.2 |
| ruby | ruby 2.6.10p210 |
| node | v25.2.1 |

## Results

| benchmark | exercises | pluma-vm | wasm (null) | wasm (drc) | python3 | ruby | node | vm vs best | wasm vs best | output |
|---|---|--:|--:|--:|--:|--:|--:|--:|--:|:--:|
| `fib` | naive recursion | 0.55 | 0.07 | 0.05 | 0.27 | 0.30 | 0.05 | 11.0x | 1.0x | ok |
| `mandelbrot` | float64 escape loop | 1.13 | 0.14 | 1.91 | 0.39 | 0.70 | 0.06 | 18.8x | 31.8x | ok |
| `primes` | integer trial division | 1.45 | 0.18 | 2.54 | 0.52 | 0.58 | 0.05 | 29.0x | 50.8x | ok |
| `sort` | sort + checksum | 1.42 | 0.25 | 1.89 | 0.04 | 0.07 | 0.06 | 35.5x | 47.2x | ok |
| `dict` | hash-map tally | 0.44 | 0.17 | 3.18 | 0.06 | 0.09 | 0.04 | 11.0x | 79.5x | ok |
| `string` | join / split / upcase | 0.74 | 0.15 | 1.10 | 0.02 | 0.07 | 0.04 | 37.0x | 55.0x | ok |
| `tree` | build + fold a tree | 1.21 | 0.45 | 1.92 | 3.35 | 0.96 | 0.11 | 11.0x | 17.5x | ok |
| `collections` | map / filter / fold | 0.64 | 0.19 | 0.68 | 0.12 | 0.15 | 0.14 | 5.3x | 5.7x | ok |

One-time cost to compile all 8 benchmarks to WasmGC artifacts: **0.04s** total (not included in the per-run `wasm` times).

## How to read this

- Pluma ships **two backends over one IR**, and the WasmGC artifact's speed
  depends heavily on the collector it runs under, so it appears three times:
  - `pluma-vm` — `pluma run <src>`, the reference VM interpreter. The time
    includes front-end compilation, because that is what the dev loop costs
    every run.
  - `wasm (null)` — the WasmGC artifact (`pluma build` once, then
    `pluma run <out>.wasm`) run under wasmtime's **null collector**:
    allocate, never free. This is a **no-GC floor** — the fastest the artifact
    can possibly go, but it OOMs any long-lived program, so it is a best-case
    bound and **not a real deploy configuration**.
  - `wasm (drc)` — the *same* artifact under wasmtime's **deferred-reference-
    counting collector**, the only real WasmGC collector wasmtime ships. This is
    the **deploy ceiling**. It is costly here because Pluma's IR boxes every
    value (every `int` is a heap object), so reference counting churns on every
    transient — the worst-fit collector for this allocation pattern. A tracing /
    generational collector (which wasmtime does not yet offer for WasmGC) would
    bulk-free instead and land much closer to the floor. **The true deploy cost
    sits between `null` and `drc`**; until wasmtime ships a tracing GC, `drc` is
    what a deploy actually pays.
  - The one-time build cost is reported separately above; `build once, run many`,
    so the per-run times measure *executing* the artifact, not compiling it.
- `vm vs best` / `wasm vs best` divide a Pluma time by the fastest competitor's
  time (greater than 1× means Pluma is slower; less than 1× means faster).
  `wasm vs best` uses the `drc` number — the deploy reality, not the no-GC floor.
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
