# Pluma runtime conformance

Pluma compiles one IR to three backends. The **VM** is the reference/oracle; the
**WasmGC** (server) and **JS** (browser) deploy backends are run on every `tests/run`
fixture and diffed against the VM. This file is the living coverage matrix, regenerated
by `just conformance` (perf numbers live in `target/conformance/`, not here).

Corpus: **195** run fixtures (compile-error fixtures excluded — they belong to the
VM run-snapshot suite).

| Backend | Match | Diverge | Skipped | Coverage |
|---|---:|---:|---:|---:|
| VM (oracle) | 195 | — | — | reference |
| WasmGC | 194 | 0 | 1 | 194/195 |
| JS | 158 | 0 | 37 | 158/195 |

## WasmGC skips

### unsupported (1)

- _wasm::emit rejected (2 diag)_ — builtin-unknown-tag

## JS skips

### denied (9)

- _53-bit int precision (raw i64 hash)_ — bare-trait-methods
- _VM-only unknown-builtin negative test_ — builtin-unknown-tag
- _debug call-site prefix not wired_ — debug-passthrough
- _no tail-call optimization yet_ — deep-recursion
- _wire codec deferred on the client_ — wire-dict, wire-fingerprint, wire-polymorphic, wire-recursive, wire-roundtrip

### gated (15)

- _`core.io` is not available on the browser target — it needs host capabilities [Fs, Process] this platform does not provide._ — fail-direct, io-append-delete, io-bytes-append, io-bytes-non-utf8, io-bytes-roundtrip, io-files, io-make-dir, io-print, io-read-all, io-read-dir, io-read-eof, io-read-lines, io-read-missing, io-write-bytes, time-basics

### unsupported (13)

- _js: Await is out of scope (sync backend only)_ — scope-both, scope-deadline, scope-handle-param, scope-race, task-combinators, task-combinators-concurrent, task-defer, task-fail, task-loop, task-loop-bind, task-shielded, task-trait-poly, task-try-chain

