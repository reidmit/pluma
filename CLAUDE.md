# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

This repo (named `pencil` on disk) implements **Pluma**, a small statically-typed functional language. The CLI binary is named `pluma`, source files use the `.pa` extension. The language is documented in `REFERENCE.md`; rough roadmap phases live in `README.md`.

## Workspace layout

Cargo workspace (see `Cargo.toml`):

- `compiler/` — the language frontend: tokenizer, parser, analyzer, types, diagnostics. The crate's public surface (`lib.rs`) re-exports `Compiler`, `Diagnostic`, `Module`/`ModuleExports`, `Tokenizer`, `Token`, and module-name/version constants; `ast` and `types` are `pub mod` (consumed by codegen). Other modules (`analyzer`, `parser`, etc.) are private.
- `codegen/` — lowers the typed AST into VM bytecode. `codegen::compile(&compiler)` returns a `vm::Program` ready to execute.
- `vm/` — bytecode VM that executes the compiled program. `VM::new(program).run()` is the entry point. `print` writes through a configurable `StdoutSink` (process stdout by default; tests inject a `Buffer` sink). `vm::stdlib::register_compiler` seeds the analyzer with the native module types (`core.regex`, `core.list`, `core.math`).
- `cli/` — command dispatcher. `run`, `tokenize`, `analyze` are wired; `build` is `todo!()`. `tokenize` and `analyze` are debug-build only (they dump Debug-format output of types whose Debug is gated on `debug_assertions`).
- `lsp/` — language server, packaged for VS Code via the extension in `vsix/`.
- `pluma-tests/` — integration tests (snapshot-based) for the analyzer and the VM. See "Testing" below.
- `bench/` — microbench runner that times each `benchmarks/programs/<name>/main.pa` through the VM.

## Common commands

The project uses `just` (run `brew install just` if missing). Run `cargo build` directly for general compilation; use the recipes below for project-specific workflows.

```
just tokenize <path>       # cargo run --bin cli -- tokenize <path>          (debug only)
just analyze <path>        # cargo run --bin cli -- analyze <path>           (debug only)
just run <path>            # cargo run --bin cli -- run <path>
just build-release         # cargo build --release --bin cli
just test                  # cargo test -p pluma-tests
just test-write            # INSTA_UPDATE=always cargo test -p pluma-tests   (accept all snapshot changes)
just vs-extension          # build LSP + extension, launch VS Code dev host pointed at ./tests
just site                  # serve site/ via zola on port 7586
```

For interactive snapshot review (preferred over `just test-write`), use `cargo insta review`. Filter tests with the normal `cargo test` filter syntax: `cargo test -p pluma-tests hello`.

The CLI accepts either a file path (with or without `.pa`) or a directory containing `main.pa` — see `get_root_dir_and_module_name` in `compiler/src/compiler.rs`.

## Testing

Fixtures live under `tests/analyze/<name>/main.pa` (and optionally additional `.pa` files for multi-module cases) and `tests/run/<name>/main.pa`. Each fixture has an `analyze.snap` or `run.snap` next to its `main.pa` — an insta snapshot file with a 3-line YAML header followed by the captured output.

- **`tests/analyze/`** fixtures run the compiler frontend in-process and snapshot `{:#?}` of the typed `Module` (or formatted diagnostics on failure).
- **`tests/run/`** fixtures compile, lower to bytecode via `codegen::compile`, then call `vm::VM::run()` with a `vm::StdoutSink::Buffer`, and snapshot a combined `status / stdout / stderr` block.

Both harnesses live in `pluma-tests/tests/{analyze,run}.rs`. `datatest-stable` generates one `#[test]` per fixture by scanning the directory for `main.pa`. Tests set cwd to the workspace root so the `Module` Debug impl renders paths as `tests/analyze/<name>/main.pa` (portable across checkouts).

When changing analyzer/parser output or VM behavior, regenerate snapshots with `cargo insta review` (interactive accept/reject) or `just test-write` (accept all). Don't hand-edit `.snap` files. To add a new test, create the fixture directory + `main.pa`, run `just test-write`, and review the generated snapshot.

## Compiler architecture

`Compiler` (in `compiler/src/compiler.rs`) is the orchestration entry point. It owns a `HashMap<String, Module>`, a `HashMap<String, ModuleExports>` cache (see below), and a `Vec<Diagnostic>`. It exposes `tokenize()` and `check()`. `check()` runs through parsing + type analysis and returns the typed entry `Module`; the bytecode/run pipeline lives downstream in the `codegen` and `vm` crates.

`check()` does a DFS load (`load_module`): for each module, it parses, then recursively loads anything in its `uses`, then analyzes the module itself (so dependencies are always analyzed first). A `visiting` set catches import cycles and reports them as a `Diagnostic`. After analysis, the module's `exports: Option<ModuleExports>` is populated from its top-level defs and cached for any later importer.

`ModuleExports` (in `module.rs`) is a three-way map: `values` for top-level value defs (and alias constructor functions), `aliases` for alias type defs (name → resolved type), `enums` for enum type defs (name → ordered list of variants). The analyzer's `set_imports` takes both this and a parallel local-name → fully-qualified-module-name map so qualified enum types can be reconstructed at use sites.

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
