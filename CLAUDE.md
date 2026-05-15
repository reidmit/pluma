# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

This repo (named `pencil` on disk) implements **Pluma**, a small statically-typed functional language. The CLI binary is named `pluma`, source files use the `.pa` extension. The language is documented in `REFERENCE.md`; rough roadmap phases live in `README.md`.

## Workspace layout

Cargo workspace with three crates (see `Cargo.toml`):

- `compiler/` — the language frontend: tokenizer, parser, analyzer, types, diagnostics. The crate's public surface (`lib.rs`) re-exports `Compiler`, `Diagnostic`, `Tokenizer`, `Token`, and module-name/version constants. Internals (`ast`, `analyzer`, `parser`, `types`, etc.) are private modules.
- `cli/` — thin command dispatcher around `Compiler` (`tokenize`, `analyze`; `run` and `build` are `todo!()`).
- `lsp/` — language server, packaged for VS Code via the extension in `vsix/`.

## Common commands

The project uses `just` (run `brew install just` if missing). Run `cargo build` directly for general compilation; use the recipes below for project-specific workflows.

```
just tokenize <path>       # cargo run --bin cli -- tokenize <path>
just analyze <path>        # cargo run --bin cli -- analyze <path>
just test                  # run snapshot tests via scripts/test.py
just test-write <name>/analyze   # regenerate analyze.out + analyze.err for a test
just test-write <name>/run       # regenerate run.out + run.err (currently disabled in runner)
just vs-extension          # build LSP + extension, launch VS Code dev host pointed at ./tests
just site                  # serve site/ via zola on port 7586
```

The `just test` recipe doesn't accept a filter — to run a subset, invoke `python3 scripts/test.py <substring>` directly.

The CLI accepts either a file path (with or without `.pa`) or a directory containing `main.pa` — see `get_root_dir_and_module_name` in `compiler/src/compiler.rs`.

## Testing

Snapshot tests live under `tests/<test-name>/main.pa` with paired `analyze.out` / `analyze.err` (and optionally `run.out` / `run.err`) capturing expected stdout/stderr. `scripts/test.py` shells out to `cargo run --bin cli -- analyze <module>` per test and prints a colored diff on mismatch. A test missing an expected output file is reported as `skipped`, not failed.

When changing analyzer/parser output, regenerate snapshots with `just test-write <name>/analyze` — don't hand-edit `.out`/`.err` files. The `run.*` test cases are commented out in `scripts/test.py` because the CLI's `run` subcommand is unimplemented.

## Compiler architecture

`Compiler` (in `compiler/src/compiler.rs`) is the orchestration entry point. It owns a `HashMap<String, Module>`, a `HashMap<String, HashMap<String, Type>>` of cached module exports, and a `Vec<Diagnostic>`. It exposes `tokenize()` and `check()`. There is no `build()`/`run()` backend yet — `check()` runs through parsing + type analysis and returns the typed entry `Module`.

`check()` does a DFS load (`load_module`): for each module, it parses, then recursively loads anything in its `uses`, then analyzes the module itself (so dependencies are always analyzed first). A `visiting` set catches import cycles and reports them as a `Diagnostic`. After analysis, the module's `exports: Option<HashMap<String, Type>>` is populated from each top-level value def's inferred type and cached for any later importer.

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

- **Enums are nominal.** `Type::Enum(String)` carries only the enum's name; structural variant info (`Vec<(variant_name, params)>`) lives in `Analyzer::enum_defs`. This is what lets `def tree enum { node int tree tree }` type-check — without it, recursive enum types hit the occurs check.
- **`FieldAccess` is overloaded** in `constrain_expr`. The handler checks, in order: imported-module namespace lookup, enum variant access, then falls back to a `PartialRecord` constraint for actual record fields.
- **Variant patterns disambiguate by subject type.** `resolve_variant_pattern` looks up a name in the subject's enum when the subject type is concrete; otherwise it does a global lookup across `enum_defs` and reports `AmbiguousVariant` if more than one enum has that name. Two helpers split the work: `find_variant_in_enum` and `find_variant_globally`.
- **`when` is exhaustiveness-checked** in `annotate_expr` (post-substitution, so the subject type is known). `Type::Bool` and `Type::Enum` are checked structurally; other subject types currently rely on a `_` catch-all and are otherwise skipped.
- **`let` bindings with concrete-typed values bind monomorphically** (`Scheme::Forall(vec![], ty)`) rather than going through Gen/Inst. Lets the value's resolved type (e.g. `Type::Enum(color)`) be visible at constraint-gen time for downstream pattern resolution. Polymorphic values still take the Gen/Inst path.
- **Cross-module values are instantiated per use** via `Analyzer::instantiate`, which walks the imported type and replaces every free `Type::Var` with a fresh one. Gives per-call-site polymorphism for imported functions.
- **The unifier has an `Enum ~ Enum` case** that succeeds iff the names match — there's no structural comparison. `Module`'s `Debug` impl prints comments through a `BTreeMap` so the test snapshot output is stable.

## Conventions

- **Tabs**, not spaces. Run `cargo fmt` (project uses `rustfmt.toml`).
- Diagnostics are accumulated, not raised — pass `&mut Vec<Diagnostic>` rather than returning `Result` for non-fatal issues.
- AST node types are split one-per-file under `ast/`; add new nodes by adding a module and re-exporting from `ast/mod.rs`. The `r#use` and `r#enum` files use the raw-identifier prefix because `use` and `enum` are Rust keywords.
- `Module::ast` is an `Option<ModuleNode>` populated by `parse`. The analyzer mutates the AST in place to attach inferred types — don't clone it out.
- `Compiler` derives `Debug` only in debug builds (`#[cfg_attr(debug_assertions, derive(Debug))]`); `analyze` output uses `{:#?}` of the `Module`, so `Debug` impls for AST nodes are part of the test surface.
