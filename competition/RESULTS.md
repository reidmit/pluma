# Pluma vs Python, Ruby, and Node.js — benchmark results

_Best of 5 runs, wall-clock seconds (lower is better). Generated 2026-06-01 16:09:47 PDT._

Correctness: all four implementations agreed on every output.

## Environment

| component | version |
|---|---|
| host | `Darwin arm64` |
| pluma | git `30d8639` (release build) |
| python3 | Python 3.11.2 |
| ruby | ruby 2.6.10p210 |
| node | v25.2.1 |

## Results

| benchmark | exercises | pluma | python3 | ruby | node | pluma vs best | output |
|---|---|--:|--:|--:|--:|--:|:--:|
| `fib` | naive recursion | 0.48 | 0.27 | 0.29 | 0.05 | 9.6x | ok |
| `mandelbrot` | float64 escape loop | 1.06 | 0.38 | 0.70 | 0.05 | 21.2x | ok |
| `primes` | integer trial division | 1.27 | 0.49 | 0.58 | 0.04 | 31.8x | ok |
| `sort` | sort + checksum | 1.61 | 0.04 | 0.07 | 0.05 | 40.2x | ok |
| `dict` | hash-map tally | 0.41 | 0.05 | 0.09 | 0.03 | 13.7x | ok |
| `string` | join / split / upcase | 1.23 | 0.02 | 0.06 | 0.03 | 61.5x | ok |
| `tree` | build + fold a tree | 1.21 | 3.22 | 0.95 | 0.11 | 11.0x | ok |
| `collections` | map / filter / fold | 0.73 | 0.12 | 0.15 | 0.14 | 6.1x | ok |

## How to read this

- Times include process startup; for Pluma they also include front-end
  compilation — the real cost of running the program.
- `pluma vs best` is Pluma's time divided by the fastest competitor's time
  (greater than 1× means Pluma is slower; less than 1× means Pluma is faster).
- `output` = `ok` means all four printed byte-identical results; `MISMATCH`
  means they disagreed and the row should not be trusted.
- This compares **idiomatic code in each language**. `core.dict` is a persistent,
  structurally-shared map (O(log n) insert, immutable, insertion-ordered);
  `list.sort` is a Pluma-level merge sort and the string ops are Pluma-level too,
  versus the other languages' native mutable maps and C-level sort/string routines.
- Where a competitor finishes in well under ~0.1 s it is essentially measuring
  interpreter startup, not the workload.
- Regenerate with `competition/run.sh [RUNS]`.
