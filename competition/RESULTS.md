# Pluma vs Python, Ruby, and Node.js — benchmark results

_Best of 5 runs, wall-clock seconds (lower is better). Generated 2026-06-02 21:27:24 PDT._

Correctness: every implementation agreed on every output.

## Environment

| component | version |
|---|---|
| host | `Darwin arm64` |
| pluma | git `761fd1b` (release build) |
| python3 | Python 3.11.2 |
| ruby | ruby 2.6.10p210 |
| node | v25.2.1 |

## Results

| benchmark | exercises | pluma-v8 | python3 | ruby | node | v8 vs best | output |
|---|---|--:|--:|--:|--:|--:|:--:|
| `fib` | naive recursion | 0.03 | 0.27 | 0.31 | 0.05 | 0.6x | ok |
| `mandelbrot` | float64 escape loop | 0.04 | 0.39 | 0.71 | 0.05 | 0.8x | ok |
| `primes` | integer trial division | 0.04 | 0.49 | 0.58 | 0.05 | 0.8x | ok |
| `sort` | sort + checksum | 0.07 | 0.03 | 0.07 | 0.06 | 2.3x | ok |
| `dict` | hash-map tally | 0.03 | 0.06 | 0.09 | 0.04 | 0.8x | ok |
| `string` | join / split / upcase | 0.04 | 0.02 | 0.07 | 0.04 | 2.0x | ok |
| `tree` | build + fold a tree | 0.52 | 3.27 | 0.95 | 0.12 | 4.3x | ok |
| `collections` | map / filter / fold | 0.11 | 0.12 | 0.15 | 0.14 | 0.9x | ok |

One-time cost to compile all 8 benchmarks to WasmGC artifacts: **0.56s** total (not included in the per-run `pluma-v8` times).

Regenerate with `competition/run.sh [RUNS]`.
