# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repo.

## Project

This repo implements **Pluma**, a small statically-typed functional language. CLI binary `pluma`, source files `.pa`. The language reference and docs are the in-app pages of `website/` — the Pluma fullstack app served at pluma.fun; `website/reference.pa` is the full reference.

Pluma has **one backend**: the **WasmGC** backend. `pluma run` and `pluma test` both compile to WasmGC and run under V8 via the `host` crate — run what you deploy. Every builtin lowers to wasm; the only surface the backend can't emit is the browser-only `std/web/dom` (unbuilt). "Server" and "browser" are host-capability *profiles* (`compiler::Platform`), not separate backends.

Crate layout is a normal Cargo workspace — frontend (`compiler`: tokenize → parse → HM-style analyze) → `ir` → `wasm` backend → `host` runtime, plus `cli`, `lsp`, `tests`. Read `Cargo.toml` and each crate's `lib.rs` for specifics rather than relying on a description here. Design-of-record notes live in `notes/` (ASYNC, IR, FULLSTACK, FRONTEND, DEPLOY, NET, RECORDS, …).

## Pluma syntax

Pluma's syntax has some unique quirks. When in doubt, mirror a fixture in `tests/run/` (`grep tests/run/*/main.pa` for a working example); the full syntax is documented in `website/reference.pa`. The traps worth memorizing:

- **Calls are uncurried and mostly paren-free.** Single arg: `print x`, `fact 5`. Multiple: `add x y`. Parens only group sub-expressions: `print (fact 5)`. `add 5` is an arity error, not partial application — wrap it: `fun y { add 5 y }`.
- **Zero-arg calls use `()`, not `{}`.** `dict.empty ()`, `io.read ()`. `{}` is a block.
- **Function literals put args bare before the brace:** `fun x { x + 1 }`, `fun x y { x + y }`, zero-arg `fun { ... }`. Top-level: `def name = fun args { body }`.
- **Type annotations use `:: TYPE`:** `def f :: fun (list a) -> int = ...`, `let xs :: list int = []`. Parens wrap compound type args (`fun (option int) -> int`).
- **Matching:** refutable `if subj is pat { ... } else { ... }` (the `else` is the no-match case). The `is pat` is **optional** and defaults to `is true`, so `if x > 10 { ... }` is a plain boolean condition; a `{` in the subject always opens the body, so parenthesize a record literal passed in the subject (`if f ({ ... }) { ... }`). Same for `while`. `when c is red { ... } is green { ... }` chains arms — only the first carries the subject, and each arm still needs its `is pat`. `when` is exhaustiveness-checked.
- **`??` unwraps an `option`/`result` to a default:** `(dict.lookup m k) ?? 0`. Lazy, right-associative; the recovering dual of `try` (which propagates failure).
- **String interpolation `"$(expr)"`** needs explicit `to-string` for non-strings: `"n = $(to-string n)"`.
- **Triple-quoted `"""..."""` are multi-line block strings.** Opening `"""` must be followed by a newline; the closing `"""`'s indentation sets the left margin stripped from every line. Quotes need no escaping inside; `$(...)` interpolation still works. The formatter preserves the triple-quoted form (it never auto-converts a regular `"...\n..."` into one).
- **`def` is top-level, `let` is local;** `let` patterns must be irrefutable. Top-level defs are **private by default** — prefix `public` to export (`opaque` exports an enum's type but hides its constructors).
- **Imports: `use core/foo`** (path segments separated by `/`, not `.`). `ref`/`option`/`result` are auto-imported; `std/task` is **not** — `use std/task` to name any `task.*` function.
- **Tests are a library, not syntax.** A `*.test.pa` file exports `def tests :: test.suite = [ test.case "name" (fun { ... }), ... ]` built from `std/test`, with assertions from `std/assert`. `pluma test` discovers and runs them under V8.

Also in the surface (see fixtures/docs): list & record spread (`[1, ...xs]`, `{ ...base, field: v }`), duration literals (`2m20s`), enums, `defer` cleanup, async (`task`/`scope`/`try`, no `async`/`await` keyword), and arithmetic overloaded over int/float with **no implicit promotion** (`2 + 3.5` is a type error).

## Commands

`just` drives project workflows (`brew install just` if missing); `cargo build` for plain compilation. Read the `justfile` for the full set — the common ones:

```
just run <path>         # run a .pa file or dir containing main.pa
just test               # cargo test -p tests  (analyze + run + format)
just test-write         # accept all snapshot changes (or use: cargo insta review)
```

Tests are insta snapshots under `tests/<suite>/<name>/`: `analyze/` pins the frontend (parse/type — happy *and* error cases), `run/` + `run-fail/` compile to WasmGC and run on V8 (snapshotting status/stdout/stderr), `format/` pins formatter idempotence. With one backend, the `run/` snapshots are the regression guard. Reject-at-compile-time cases belong in `analyze/`, not `run/`. Don't hand-edit `.snap` files — regenerate with `cargo insta review` or `just test-write`.

## Conventions

- **Tabs**, not spaces (`cargo fmt`; config in `rustfmt.toml`).
- Diagnostics are **accumulated, not raised** — thread `&mut Vec<Diagnostic>` rather than returning `Result` for non-fatal issues.
- The analyzer **mutates the AST in place** to attach inferred types — don't clone it out.
- **No ephemeral references in code comments.** Never cite internal docs (`notes/X.md`, `RPC.md §7`), phase/slice/milestone labels ("Phase 3", "Layer 2", "slice 9"), or section pointers. Those are meaningful only during the initial implementation; the code is the long-lived source of truth. Describe what the code does and why, self-contained — inline the one-sentence rationale instead of pointing at a doc.
