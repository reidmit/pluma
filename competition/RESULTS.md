# Pluma vs Grain, Python, Ruby, Node, Bun, and Deno — benchmark results

_Best of 5 runs, wall-clock seconds (lower is better). Generated 2026-06-15 20:37:38 PDT._

Correctness: every implementation agreed on every output.

## Environment

| component | version |
|---|---|
| host | `Darwin arm64` |
| pluma | git `f5d24a90` (release build) |
| python3 | Python 3.11.2 |
| ruby | ruby 2.6.10p210 |
| node | v25.2.1 |
| bun | 1.3.3 |
| deno | deno 1.31.3 (release, aarch64-apple-darwin) |
| grain | 0.7.2 |

## Results

| benchmark | exercises | pluma-v8 | pluma-src | grain-wasm | grain-src | python3 | ruby | node | bun | deno | vs best | output |
|---|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--:|
| `fib` | naive recursion | 0.07 | 0.07 | 2.31 | 4.99 | 0.27 | 0.30 | 0.05 | 0.02 | 0.04 | 3.5x | ok |
| `mandelbrot` | float64 escape loop | 0.07 | 0.07 | 10.93 | 14.74 | 0.40 | 0.71 | 0.05 | 0.03 | 0.04 | 2.3x | ok |
| `primes` | integer trial division | 0.07 | 0.07 | 6.96 | 10.17 | 0.50 | 0.59 | 0.05 | 0.02 | 0.04 | 3.5x | ok |
| `sort` | sort + checksum | 0.07 | 0.08 | 9.36 | 12.56 | 0.04 | 0.07 | 0.06 | 0.03 | 0.05 | 2.3x | ok |
| `dict` | hash-map tally | 0.10 | 0.12 | 1.31 | 4.73 | 0.06 | 0.10 | 0.04 | 0.02 | 0.03 | 5.0x | ok |
| `string` | join / split / upcase | 0.07 | 0.09 | 1.02 | 4.22 | 0.02 | 0.07 | 0.04 | 0.01 | 0.03 | 7.0x | ok |
| `tree` | build + fold a tree | 0.19 | 0.20 | 3.41 | 6.12 | 3.50 | 1.00 | 0.12 | 0.09 | 0.10 | 2.1x | ok |
| `collections` | map / filter / fold | 0.14 | 0.15 | 2.16 | 5.30 | 0.12 | 0.16 | 0.14 | 0.09 | 0.15 | 1.6x | ok |
| `interp` | AST interpreter | 0.16 | 0.16 | 11.13 | 14.51 | 2.61 | 3.66 | 0.36 | 0.38 | 0.35 | 0.5x | ok |
| `nbody` | n-body float sim | 0.09 | 0.11 | 12.53 | 17.34 | 0.74 | 1.07 | 0.04 | 0.03 | 0.03 | 3.0x | ok |
| `sieve` | sieve of Eratosthenes | 0.23 | 0.23 | 8.96 | 12.54 | 2.64 | 1.30 | 0.06 | 0.04 | 0.05 | 5.8x | ok |
| `json` | JSON round-trip | 0.19 | 0.22 | 5.82 | 9.54 | 0.06 | 0.18 | 0.05 | 0.03 | 0.06 | 6.3x | ok |
| `regex` | regex scan + extract | 0.14 | 0.18 | 5.38 | 9.59 | 0.05 | 0.14 | 0.04 | 0.02 | 0.03 | 7.0x | ok |

`pluma-v8` runs a prebuilt WasmGC artifact (`pluma run <out>.wasm`); `pluma-src` runs from source
(`pluma run <prog>.pa`), so its time folds the full tokenize/parse/analyze/IR/wasm pipeline into every
run — the gap between the two columns is that compile cost. `grain-wasm` is the analogous Grain deploy
artifact (`grain compile --release`, then `grain run <out>.gr.wasm`); `grain-src` is `grain <prog>.gr`,
compiling and running from source each invocation. `vs best` compares the deploy artifact (`pluma-v8`)
against the fastest other language (including `grain-wasm`).

One-time build cost, summed across all 13 benchmarks and **not** included in the per-run times above:
Pluma compile-to-WasmGC **1.51s**; Grain compile-to-wasm **294.86s**.

Regenerate with `competition/run.sh [RUNS]`.
