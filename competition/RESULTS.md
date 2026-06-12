# Pluma vs Python, Ruby, Node, Bun, and Deno — benchmark results

_Best of 5 runs, wall-clock seconds (lower is better). Generated 2026-06-12 08:52:12 PDT._

Correctness: every implementation agreed on every output.

## Environment

| component | version |
|---|---|
| host | `Darwin arm64` |
| pluma | git `7310738` (release build) |
| python3 | Python 3.11.2 |
| ruby | ruby 2.6.10p210 |
| node | v25.2.1 |
| bun | 1.3.3 |
| deno | deno 1.31.3 (release, aarch64-apple-darwin) |

## Results

| benchmark | exercises | pluma-v8 | pluma-src | python3 | ruby | node | bun | deno | vs best | output |
|---|---|--:|--:|--:|--:|--:|--:|--:|--:|:--:|
| `fib` | naive recursion | 0.03 | 0.03 | 0.27 | 0.30 | 0.05 | 0.02 | 0.04 | 1.5x | ok |
| `mandelbrot` | float64 escape loop | 0.04 | 0.04 | 0.39 | 0.70 | 0.05 | 0.03 | 0.04 | 1.3x | ok |
| `primes` | integer trial division | 0.04 | 0.05 | 0.49 | 0.58 | 0.05 | 0.02 | 0.04 | 2.0x | ok |
| `sort` | sort + checksum | 0.07 | 0.07 | 0.04 | 0.07 | 0.06 | 0.03 | 0.05 | 2.3x | ok |
| `dict` | hash-map tally | 0.07 | 0.08 | 0.05 | 0.09 | 0.04 | 0.02 | 0.03 | 3.5x | ok |
| `string` | join / split / upcase | 0.04 | 0.06 | 0.02 | 0.07 | 0.03 | 0.01 | 0.03 | 4.0x | ok |
| `tree` | build + fold a tree | 0.52 | 0.53 | 3.29 | 0.96 | 0.11 | 0.08 | 0.09 | 6.5x | ok |
| `collections` | map / filter / fold | 0.12 | 0.13 | 0.12 | 0.16 | 0.14 | 0.09 | 0.15 | 1.3x | ok |
| `interp` | AST interpreter | 0.17 | 0.17 | 2.54 | 3.63 | 0.35 | 0.37 | 0.34 | 0.5x | ok |
| `nbody` | n-body float sim | 1.37 | 1.38 | 0.73 | 1.06 | 0.04 | 0.03 | 0.03 | 45.7x | ok |
| `sieve` | sieve of Eratosthenes | 0.23 | 0.23 | 2.62 | 1.30 | 0.06 | 0.04 | 0.05 | 5.8x | ok |
| `json` | JSON round-trip | 0.25 | 0.28 | 0.06 | 0.16 | 0.05 | 0.02 | 0.04 | 12.5x | ok |
| `regex` | regex scan + extract | 0.46 | 0.48 | 0.04 | 0.12 | 0.04 | 0.01 | 0.03 | 46.0x | ok |

`pluma-v8` runs a prebuilt WasmGC artifact (`pluma run <out>.wasm`); `pluma-src` runs from source
(`pluma run <prog>.pa`), so its time folds the full tokenize/parse/analyze/IR/wasm pipeline into every
run — the gap between the two columns is that compile cost. `vs best` compares the deploy artifact
(`pluma-v8`) against the fastest other language.

One-time build cost, summed across all 13 benchmarks and **not** included in the per-run times above:
Pluma compile-to-WasmGC **0.13s**.

Regenerate with `competition/run.sh [RUNS]`.
