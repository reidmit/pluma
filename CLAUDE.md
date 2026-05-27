# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

This repo (named `pencil` on disk) implements **Pluma**, a small statically-typed functional language. The CLI binary is named `pluma`, source files use the `.pa` extension. The language is documented under `site/content/docs/` (the zola-built docs site); rough roadmap phases live in `README.md`.

## Pluma syntax — common gotchas

When writing `.pa` code, these are the traps that don't match other languages' intuitions. When in doubt, mirror a fixture in `tests/run/`.

- **Zero-arg calls use `()`, not `{}`.** `do-it ()`, `dict.empty ()`, `io.read ()`, `random.int ()`. `{}` is a block expression — `do-it {}` parses as calling `do-it` with a block, which is a type error.
- **Pluma is uncurried.** `add 5` is an arity error, not partial application. To partially apply, wrap: `fun y { add 5 y }`.
- **Function literals: args bare before the brace.** `fun x { x + 1 }`, `fun x y { x + y }`. Zero-arg form is `fun { ... }`. Top-level functions: `def name = fun args { body }`.
- **Single-arg calls take no parens.** `print x`, `fact 5`, `to-string n`. Parens are only for grouping a sub-expression: `print (fact 5)`, `print (1 + 2)`.
- **Refutable `if`: `if subj is pat { ... } else { ... }`.** The `else` branch is the no-match case. Example: `if r is ok v { v } else { 0 }`.
- **`when` chains multiple `is` branches.** `when c is red { "r" } is green { "g" } is blue { "b" }`. No commas; only the first arm has the subject (`when c is …`), subsequent arms start with bare `is`.
- **Type annotations use `:: TYPE`.** Top-level: `def length :: fun (list a) -> int = built-in "list-length"`. Local: `let xs :: list int = []`. Parens wrap compound type args inside a `fun` signature (`fun (list a) -> int`, `fun (option int) -> int`).
- **Arithmetic operators `+ - * / %` are overloaded over int and float** via the `numeric` trait (and a heuristic for `%`); there are no dotted float operators. Both operands must be the same type — `2 + 3.5` is a type error (no implicit int/float promotion). `<` `>` `<=` `>=` dispatch through `ord`; `== !=` are structural. `++` is string concat.
- **`??` unwraps an `option`/`result`** to a bare value, else its right-hand default: `opt ?? fallback`, `(dict.lookup m k) ?? 0`. Lazy (the default is only evaluated on `none`/`err`) and right-associative, so `a ?? b ?? c` chains. It's the recovering dual of `try` (which propagates the failure): `??` desugars to `option.or-else`/`result.or-else`, exactly as `try` desugars to `.then`. For `result`, the `err` value is discarded.
- **List literals support `...` spread.** `[1, ...xs, 2, ...ys]` — a `...expr` element splices in another list (its element type must match). Spreads can appear at any position, any number of times; prefer this over `list.concat [head] tail`. Note the asymmetry with list *patterns*, where `...rest` is single and trailing-only (`[head, ...tail]`).
- **String interpolation: `"$(expr)"`.** Non-string values need explicit `to-string`: `"n = $(to-string n)"`. A bare string variable interpolates directly: `"hi $(name)"`.
- **Duration literals: digits + a time unit, no space.** `1s`, `250ms`, `2m20s`, `3h2m10s`, `100d` are `duration` values (the prelude type behind `core.time`) — no `use core.time` needed to write one. Units are `d h m s ms us ns`; segments must each appear at most once and in strictly descending order (`2m20s` ok; `20s2m`/`2m3m` are parse errors). Integer components only — `1.5s` is rejected. The literal lowers straight to a constant, so it's not sugar for `time.seconds`; `time` is only needed to operate on the value. The formatter normalizes non-canonical spellings (`90s` → `1m30s`).
- **Enums.** Declare with newline-separated variants; payload follows the variant name:
  ```
  enum option a {
      some a
      none
  }
  ```
  Construct as `option.some 42` or bare `some 42` when the type is clear. Match by bare variant: `when x is some n { ... } is none { ... }`.
- **`def` is top-level; `let` is local.** Top-level bindings can't use `let`, and `let` patterns must be irrefutable (use `if`/`when` for `some`/`ok`/etc.).
- **`defer expr` runs cleanup at function exit.** A body statement (like `let`) that schedules `expr` to run when the enclosing **function** returns — on the normal path *and* when a `try` short-circuits on failure. Multiple `defer`s run LIFO; a `defer` only fires if execution reached it (so one after a short-circuiting `try`, or in an un-taken `if` branch, is skipped). `defer` itself evaluates to `nothing`. Scope-exit cleanup and the async facets (cancellation/awaited cleanup) aren't built yet — see `ASYNC.md`.
- **Top-level defs are private by default; `public`/`opaque` widen that.** A bare `def`/`enum`/`alias` is visible only inside its own module — importers can't see it (you'll get "`x` is private to module `y`"). Prefix `public` to export it (`public def map = …`, `public enum color { … }`). `opaque` is enum-only and exports the *type* while hiding its *constructors*: importers can name `mod.token` and pass values around, but can't `mod.token.mk` or pattern-match it — make values through public smart-constructor functions instead. `opaque def`/`opaque alias`/`public trait` are parse errors (traits/instances aren't on the visibility ladder; instances are always exported). Visibility only gates the analyzer's cross-module type-checking — codegen still compiles every def, so it never changes runtime behavior. The whole stdlib is explicitly `public`; mirror that when writing a module others import.
- **`task a` is the async carrier; `try` awaits it.** A `task a` is a cold, re-runnable async computation (build it with `task.return`/`task.fail`/`task.sleep`/`task.yield`, or by calling a function that awaits). `task` is the third `try` carrier: inside a function that returns a task, `try x = some-task` awaits it and the whole function is a `task` (an honest effect annotation — there's no `async`/`await` keyword). The `task.*` helpers are in `core.task`, **auto-imported** like `option`/`result` (no `use`). `?? `/`task.or-else` recover; `task.attempt` reifies failure to a `result`; `task.map`/`task.then` transform. The runtime is lazy — a script that never makes a task pays nothing. `scope`/`manual scope` (structured concurrency) and the scope-level combinators (`task.all`/`race`/…) are **not built yet** — see `ASYNC.md` for status. Mirror a `tests/run/task-*` fixture when writing async code.
- **`use core.foo` for stdlib imports.** Available modules include `core.list`, `core.dict`, `core.bytes`, `core.string`, `core.math`, `core.assert`, `core.testing`, `core.hex`, `core.base64`, `core.random`, `core.uuid`, `core.time`. `ref`, `option`, `result`, and `task` are auto-imported — don't `use` them.
- **Tests are a library, not syntax.** There is no `test` keyword. A `*.test.pa` file exports `def tests :: testing.suite = fun t { ... }` and registers cases with `t.case "name" (fun { ... })` (also `t.group`/`t.skip`/`t.focus`/`t.todo`). A case body returns a `result`: `core.assert` checks (`assert.equals`, `assert.is-true`, …) each return `ok ()`/`err msg`, and `assert.all [..]` combines several. `pluma test` discovers the files and runs them. See `compiler/src/stdlib/*.test.pa` for examples.

For unfamiliar stdlib calls, `grep tests/run/*/main.pa` for a working example rather than guessing.

## Workspace layout

Cargo workspace (see `Cargo.toml`):

- `compiler/` — the language frontend: tokenizer, parser, analyzer, types, diagnostics. The crate's public surface (`lib.rs`) re-exports `Compiler`, `Diagnostic`, `Module`/`ModuleExports`, `Tokenizer`, `Token`, and module-name/version constants; `ast` and `types` are `pub mod` (consumed by codegen). Other modules (`analyzer`, `parser`, etc.) are private.
- `codegen/` — lowers the typed AST into VM bytecode. `codegen::compile(&compiler)` returns a `vm::Program` ready to execute.
- `vm/` — bytecode VM that executes the compiled program. `VM::new(program).run()` is the entry point. `print` writes through a configurable `StdoutSink` (process stdout by default; tests inject a `Buffer` sink). `vm::stdlib::register_compiler` seeds the analyzer with any Rust-defined native module types — currently none, since every stdlib module (including `core.dict`) is a `.pa` source; the mechanism remains for any future module whose signature the `.pa` surface can't express.
- `cli/` — command dispatcher. `run`, `format`, `tokenize`, `analyze` are wired. `tokenize` and `analyze` are debug-build only (they dump Debug-format output of types whose Debug is gated on `debug_assertions`).
- `lsp/` — language server, packaged for VS Code via the extension in `vsix/` and for Zed via the extension in `zed/`. Both editor extensions are thin clients that just launch `pluma-language-server`; all features (diagnostics, hover, formatting, highlighting via semantic tokens) live in the LSP so they're shared. The `zed/` crate ships no Tree-sitter grammar and is a standalone (non-workspace) cdylib built for `wasm32-wasip1`.
- `tests/` — integration tests (snapshot-based) for the analyzer and the VM. The Cargo package is also named `tests`. Harness files live at the crate root (`tests/analyze.rs`, `tests/run.rs`) next to the fixture directories (`tests/analyze/`, `tests/run/`). See "Testing" below.
- `bench/` — microbench runner that times each `benchmarks/programs/<name>/main.pa` through the VM.

## Common commands

The project uses `just` (run `brew install just` if missing). Run `cargo build` directly for general compilation; use the recipes below for project-specific workflows.

```
just tokenize <path>       # cargo run --bin cli -- tokenize <path>          (debug only)
just analyze <path>        # cargo run --bin cli -- analyze <path>           (debug only)
just run <path>            # cargo run --bin cli -- run <path>
just build-release         # cargo build --release --bin cli
just test                  # cargo test -p tests
just test-write            # INSTA_UPDATE=always cargo test -p tests   (accept all snapshot changes)
just vs-extension          # build LSP + extension, launch VS Code dev host pointed at ./tests
just site                  # serve site/ via zola on port 7586
```

For interactive snapshot review (preferred over `just test-write`), use `cargo insta review`. Filter tests with the normal `cargo test` filter syntax: `cargo test -p tests hello`.

The CLI accepts either a file path (with or without `.pa`) or a directory containing `main.pa` — see `get_root_dir_and_module_name` in `compiler/src/compiler.rs`.

## Testing

Fixtures live under `tests/analyze/<name>/main.pa` (and optionally additional `.pa` files for multi-module cases) and `tests/run/<name>/main.pa`. Each fixture has an `analyze.snap` or `run.snap` next to its `main.pa` — an insta snapshot file with a 3-line YAML header followed by the captured output.

- **`tests/analyze/`** fixtures run the compiler frontend in-process and snapshot `{:#?}` of the typed `Module` (or formatted diagnostics on failure).
- **`tests/run/`** fixtures compile, lower to bytecode via `codegen::compile`, then call `vm::VM::run()` with a `vm::StdoutSink::Buffer`, and snapshot a combined `status / stdout / stderr` block.

Both harnesses live in `tests/{analyze,run}.rs` (registered via `path =` in `tests/Cargo.toml` so they sit alongside the fixture directories rather than in a nested `tests/tests/`). `datatest-stable` generates one `#[test]` per fixture by scanning the directory for `main.pa`. Tests set cwd to the workspace root so the `Module` Debug impl renders paths as `tests/analyze/<name>/main.pa` (portable across checkouts).

When changing analyzer/parser output or VM behavior, regenerate snapshots with `cargo insta review` (interactive accept/reject) or `just test-write` (accept all). Don't hand-edit `.snap` files. To add a new test, create the fixture directory + `main.pa`, run `just test-write`, and review the generated snapshot.

## Compiler architecture

`Compiler` (in `compiler/src/compiler.rs`) is the orchestration entry point. It owns a `HashMap<String, Module>`, a `HashMap<String, ModuleExports>` cache (see below), and a `Vec<Diagnostic>`. It exposes `tokenize()` and `check()`. `check()` runs through parsing + type analysis and returns the typed entry `Module`; the bytecode/run pipeline lives downstream in the `codegen` and `vm` crates.

`check()` does a DFS load (`load_module`): for each module, it parses, then recursively loads anything in its `uses`, then analyzes the module itself (so dependencies are always analyzed first). A `visiting` set catches import cycles and reports them as a `Diagnostic`. After analysis, the module's `exports: Option<ModuleExports>` is populated from its top-level defs and cached for any later importer.

`ModuleExports` (in `module.rs`) is a three-way map: `values` for top-level value defs (and alias constructor functions), `aliases` for alias type defs (name → resolved type), `enums` for enum type defs (name → ordered list of variants). The analyzer's `set_imports` takes both this and a parallel local-name → fully-qualified-module-name map so qualified enum types can be reconstructed at use sites.

Visibility (`ast::Visibility`, set by the parser from a leading `public`/`opaque`) is applied when the analyzer *builds* exports (`analyzer.rs`): a private def is omitted from `values`/`aliases`/`enums` and its name recorded in a `private` set (so importers can report "`x` is private" rather than a bare "not found"); an `opaque` enum is exported in `enums` with `param_count` intact but an **empty variant list**, so the type resolves while construction/pattern-matching find no variants. So filtering happens once, at the export boundary — consumers just see (or don't see) the names.

Pipeline within a single module:

1. **Tokenize** — `Tokenizer::from_source(&bytes)` (in `tokenizer.rs`) yields `Token`s from raw source bytes.
2. **Parse** — `Parser::new(bytes, tokenizer).parse_module()` produces a `ModuleNode` (which has both `uses: Vec<UseNode>` and `body: Vec<DefinitionNode>`), plus a comment map and `ParseError`s. The AST is split across `ast/*.rs` with one file per node category (`call`, `definition`, `enum`, `expr`, `fun`, `if`, `let`, `literal`, `pattern`, `use`, `when`, `while`, etc.) and a shared `ast/mod.rs` re-export list.
3. **Analyze** — `Analyzer::analyze(module)` in `compiler/src/analyzer.rs` is a Hindley-Milner-style three-phase pass:
   - `constrain(ast)` walks the AST, assigning fresh type variables and producing `Constraint`s; literals get concrete types inline.
   - `unify(&constraints)` solves for a `Substitution`.
   - The substitution is applied back to the AST, filling in the inferred types in-place (the AST is mutated).

   Built-in types (`int`, `bool`, `string`, `regex`, `float`) are seeded into the type scope at the start of `analyze`. Value/type scopes are stacks of `HashMap`s; identifiers resolve into `ValueBinding` / `TypeBinding` (in `binding.rs`). The analyzer also takes an `imports` map (local namespace name → that module's exports) via `set_imports`.

Errors flow through a single `Vec<Diagnostic>` threaded by reference through tokenize/parse/analyze. `Diagnostic` carries optional module + range and is rendered by `cli/src/printing.rs`. The error kinds live in `compiler/src/errors/` (`parse_error.rs`, `analysis_error.rs`, `usage_error.rs`).

## Type system notes

- **Enums are nominal, with module-qualified identity.** `Type::Enum(String)` carries a fully-qualified name `<defining-module>.<enum-name>` (e.g. `colors.color`). Structural variant info lives in `Analyzer::enum_defs`, keyed by that same qualified name. Both locally-defined enums and imported ones live in `enum_defs` under their qualified keys, so variant resolution is uniform. `Type`'s `Display` strips up to the last dot, so snapshots and error messages still render bare names (`color`).
- **`FieldAccess` is overloaded** in `constrain_expr`. The handler checks, in order: chained `module.enum.variant` (cross-module variant access), imported-module namespace lookup (`module.value`), local enum variant access (`enum.variant` via the type_scope binding's qualified name), then falls back to a `PartialRecord` constraint for actual record fields.
- **`module.TypeName` in type positions.** `TypeIdentifierNode` carries an optional `module: Option<IdentifierNode>` prefix populated by `parse_type_identifier`. `Analyzer::type_expr_to_type` resolves it against the importer's `imports` map (checking exported enums first, then aliases) and produces the qualified `Type::Enum` or the resolved alias type.
- **Variant patterns disambiguate by subject type.** `resolve_variant_pattern` looks up a name in the subject's enum when the subject type is concrete; otherwise it does a global lookup across `enum_defs` and reports `AmbiguousVariant` if more than one enum has that name. The global pool includes imported enums too, so bare variant patterns can match imported enums when the subject type is known.
- **`when` is exhaustiveness-checked** in `annotate_expr` (post-substitution, so the subject type is known). `Type::Bool` and `Type::Enum` are checked structurally; other subject types currently rely on a `_` catch-all and are otherwise skipped.
- **`let` bindings with concrete-typed values bind monomorphically** (`Scheme::Forall(vec![], ty)`) rather than going through Gen/Inst. Lets the value's resolved type (e.g. `Type::Enum(color)`) be visible at constraint-gen time for downstream pattern resolution. Polymorphic values still take the Gen/Inst path.
- **Cross-module values are instantiated per use** via `Analyzer::instantiate`, which walks the imported type and replaces every free `Type::Var` with a fresh one. Gives per-call-site polymorphism for imported functions.
- **The unifier has an `Enum ~ Enum` case** that succeeds iff the qualified names match. `Module`'s `Debug` impl prints comments through a `BTreeMap` so the test snapshot output is stable.

## Conventions

- **Tabs**, not spaces. Run `cargo fmt` (project uses `rustfmt.toml`).
- Diagnostics are accumulated, not raised — pass `&mut Vec<Diagnostic>` rather than returning `Result` for non-fatal issues.
- AST node types are split one-per-file under `ast/`; add new nodes by adding a module and re-exporting from `ast/mod.rs`. The `r#use` and `r#enum` files use the raw-identifier prefix because `use` and `enum` are Rust keywords.
- `Module::ast` is an `Option<ModuleNode>` populated by `parse`. The analyzer mutates the AST in place to attach inferred types — don't clone it out.
- `Compiler` derives `Debug` only in debug builds (`#[cfg_attr(debug_assertions, derive(Debug))]`); `analyze` output uses `{:#?}` of the `Module`, so `Debug` impls for AST nodes are part of the test surface.
