# Pluma vs Python, Ruby, Node, Bun, Deno, LuaJIT, and Haskell — benchmark results

_Best of 5 runs, wall-clock seconds (lower is better). Generated 2026-06-03 12:58:24 PDT._

Correctness: every implementation agreed on every output.

## Environment

| component | version |
|---|---|
| host | `Darwin arm64` |
| pluma | git `b1e0c9e` (release build) |
| python3 | Python 3.11.2 |
| ruby | ruby 2.6.10p210 |
| node | v25.2.1 |
| bun | 1.3.3 |
| deno | deno 1.31.3 (release, aarch64-apple-darwin) |
| luajit | LuaJIT 2.1.1765228720 |
| ghc | 9.4.4 |

## Results

| benchmark | exercises | pluma-v8 | python3 | ruby | node | bun | deno | luajit | haskell | vs best | output |
|---|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--:|
| `fib` | naive recursion | 0.03 | 0.27 | 0.31 | 0.05 | 0.02 | 0.04 | 0.01 | 0.02 | 3.0x | ok |
| `mandelbrot` | float64 escape loop | 0.04 | 0.40 | 0.72 | 0.05 | 0.03 | 0.05 | 0.02 | 0.03 | 2.0x | ok |
| `primes` | integer trial division | 0.04 | 0.50 | 0.59 | 0.05 | 0.02 | 0.04 | 0.03 | 0.02 | 2.0x | ok |
| `sort` | sort + checksum | 0.07 | 0.04 | 0.08 | 0.06 | 0.03 | 0.05 | 0.03 | 0.06 | 2.3x | ok |
| `dict` | hash-map tally | 0.08 | 0.06 | 0.10 | 0.04 | 0.02 | 0.03 | 0.01 | 0.10 | 8.0x | ok |
| `string` | join / split / upcase | 0.05 | 0.03 | 0.07 | 0.04 | 0.01 | 0.03 | 0.01 | 0.03 | 5.0x | ok |
| `tree` | build + fold a tree | 0.57 | 3.54 | 0.99 | 0.12 | 0.09 | 0.10 | 0.29 | 0.04 | 14.2x | ok |
| `collections` | map / filter / fold | 0.12 | 0.13 | 0.16 | 0.15 | 0.09 | 0.16 | 0.01 | 0.01 | 12.0x | ok |
| `interp` | AST interpreter | 0.17 | 2.62 | 3.72 | 0.37 | 0.39 | 0.35 | 0.39 | 0.01 | 17.0x | ok |
| `nbody` | n-body float sim | 1.37 | 0.75 | 1.09 | 0.04 | 0.03 | 0.04 | 0.06 | 0.04 | 45.7x | ok |
| `sieve` | sieve of Eratosthenes | 0.23 | 2.67 | 1.32 | 0.07 | 0.04 | 0.05 | 0.11 | 0.07 | 5.8x | ok |
| `json` | JSON round-trip | 0.98 | 0.06 | 0.17 | 0.05 | 0.02 | 0.05 | n/a | n/a | 49.0x | ok |

One-time build cost, summed across all 12 benchmarks and **not** included in the per-run times above:
Pluma compile-to-WasmGC **0.13s**; Haskell `ghc -O2` **5.13s**.

Regenerate with `competition/run.sh [RUNS]`.
