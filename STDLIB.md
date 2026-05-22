# STDLIB.md

Design and implementation plan for moving Pluma's standard library
from Rust boilerplate (`vm/src/stdlib.rs`) into `.pa` source files.
The end state: `core.list`, `core.string`, `core.io`, etc. are real
Pluma modules with doc comments, LSP-visible signatures, and a thin
`built-in "tag"` primitive linking each def to a Rust implementation.

Read top-to-bottom for the design; jump to "Implementation phases" to
execute.

## Why

Concrete capabilities this unlocks:

- **Doc comments on stdlib defs.** `core.list.map` gets `# Applies f
  to each element...` directly above its signature; hover-docs in the
  LSP just work.
- **Signatures visible to humans and tools.** Today every stdlib type
  lives as a `Type::Fun(vec![Type::String], Box::new(Type::Bool))` in
  Rust prose. As a Pluma annotation it's `fun string -> bool` —
  readable, greppable, and the same syntax users write in their own
  code.
- **One source of truth.** Today the analyzer knows stdlib types via
  `vm::stdlib::register_compiler`, codegen knows the same defs via
  `vm::stdlib::native_modules()`, and the VM dispatches via
  `Builtin` tags. Three parallel listings of the same surface. After
  this, the `.pa` file is canonical; only the tag→`Builtin` map
  remains in Rust.
- **Stdlib written in Pluma where it can be.** `option.is-some`,
  `list.is-empty`, and friends are pure-Pluma one-liners — no builtin
  needed. The migration removes them from Rust entirely.
- **LSP "go to definition" lands in readable code.** Right now it
  goes nowhere or to a `Type::Fun(...)` literal. After: it lands on
  the `.pa` source where you can read the signature and doc.

Things we deliberately do NOT include:

- **A package manager or third-party stdlib mechanism.** `core.*` is
  baked into the compiler binary via `include_str!`, same as
  `prelude.pa`. External libraries are a separate problem.
- **A new FFI / `extern` declaration form.** `built-in "tag"` is a
  regular expression, legal only on the RHS of a type-annotated
  top-level `def`. No new top-level keyword, no `extern` block.
- **Dynamic dispatch on tag strings.** The tag→`Builtin` lookup
  happens once at codegen time. Unknown tag → compile-time
  diagnostic. The VM never sees the string.
- **Removal of the `Builtin` enum.** It stays — it gives `eval.rs`
  compile-time exhaustiveness and avoids string interning on every
  call.

## What's new in the language

Two additions, both small:

### 1. Top-level type annotations on `def`

```pluma
def length :: fun (list a) -> int = built-in "list-length"
def map    :: fun (list a) (fun a -> b) -> list b = built-in "list-map"
def insert :: fun (map k v) k v -> map k v where (hash k) = built-in "map-insert"
def pi     :: float = 3.14159
def add    :: fun int int -> int = fun x y { x + y }
```

The `::` separator and `fun T1 T2 ... TN -> R` shape **already exist**
in Pluma — they're used inside `trait { ... }` method declarations and
record-type field syntax. See `tests/analyze/trait-numeric-scaffold/main.pa`:

```pluma
trait my-numeric a {
    add    :: fun a a -> a
    negate :: fun a -> a
}
```

The parser already handles `fun T1 T2 ... TN -> R` via
`parse_type_fun` in `compiler/src/parser.rs:2385`. Each atom is one
parameter (Rule A — see "What we considered" below). The missing
piece is the `:: <type>` slot on top-level `def`s; today only `def
name = expr` is accepted.

Type-annotation rules:

- `::` separates name from type (not `:` — colon is taken by
  record-field syntax like `{x: 1}`).
- Compound (generic-applied) params must be parenthesized: `fun (list
  a) -> int` is unary; bare `fun list a -> int` would be two atoms,
  i.e., two args `list` and `a` (arity error on `list`).
- Greedy generic application (`result int string` =
  `result<int, string>`) lives only in *single-type* contexts (alias
  bodies, function return types, record-field types, tuple elements).
  This is the current behavior of
  `parse_type_expression_with_generics` (`parser.rs:2343`).
- `where` constraints on a value annotation follow the same shape
  they take on trait instances today: `... where (hash k)`.

### 2. The `built-in "tag"` expression

A primitive expression. The string is a stable name resolved at
codegen time against a Rust-side table to a `Builtin` enum variant.
Legal only on the RHS of a type-annotated top-level `def`; the
annotation supplies the type (the expression itself has no inherent
type). Codegen emits `Value::Builtin(tag)` directly into the
allocated global slot — no extra closure layer.

Errors:

- Tag string not present in the table → compile-time diagnostic at
  the `built-in` expression site.
- Used without a surrounding type annotation → diagnostic ("`built-in`
  expressions require a type annotation on the enclosing def").
- Used inside an inner expression (not as the immediate RHS of a top-
  level def) → diagnostic. This keeps things simple; we can relax
  later if a real use case appears.

## The shape

Example — what `core/list.pa` looks like after migration. Compare to
the current `list_module()` in `vm/src/stdlib.rs:103-241`.

```pluma
# Returns the number of elements in the list.
def length :: fun (list a) -> int = built-in "list-length"

# True if the list has zero elements.
def is-empty :: fun (list a) -> bool = fun l { length l == 0 }

# Returns a new list with the elements in reverse order.
def reverse :: fun (list a) -> list a = built-in "list-reverse"

# Applies `f` to each element, building a new list. The element type
# may change between input and output.
def map :: fun (list a) (fun a -> b) -> list b = built-in "list-map"

# Sorts the list using the given comparison function. Pair with
# `ord.compare` to sort any list whose elements have an `ord`
# instance.
def sort :: fun (list a) (fun a a -> ordering) -> list a =
    built-in "list-sort"
```

Notice `is-empty` doesn't need a builtin at all — it's expressible in
pure Pluma. The migration is an opportunity to push these down where
possible.

## What stays in Rust

- **`Builtin` enum** in `vm/src/builtin.rs` — every tag string in any
  stdlib `.pa` file must map to exactly one variant.
- **Implementations** in `vm/src/eval.rs`. Unchanged.
- **A single tag table.** Replaces `vm::stdlib::native_modules()` and
  the `NativeDef`/`NativeConstant` types. Shape:
  ```rust
  pub fn builtin_table() -> &'static [(&'static str, Builtin)] {
      &[
          ("print",         Builtin::Print),
          ("list-length",   Builtin::ListLength),
          ("list-map",      Builtin::ListMap),
          // ... one row per Builtin variant
      ]
  }
  ```
- **The `register_native_module` path on `Compiler`** for the
  pre-evaluated *constants* (e.g. `math.pi`, `math.e`). These can't
  be expressed in pure Pluma syntax today (no float literal precision
  guarantees, no compile-time constants), so they stay as registered
  values. Or — alternative — we add a `built-in-constant "tag"` form
  later. Punt for now.

## Existing infrastructure to reuse

The prelude already proves every piece of this design works:

- **`include_str!` baking.** `compiler/src/prelude.pa` is read at
  build time via `include_str!` in `compiler/src/compiler.rs:102`
  and parsed + analyzed as the synthetic `__prelude__` module before
  any user module loads. Stdlib modules follow the same pattern.
- **Pluma-defined trait instances.** `prelude.pa` already declares
  parametric instances like `implement ord (option a) where (ord a)`,
  proving `where`-constrained instance bodies work end-to-end from
  Pluma source.
- **Native-module registration.** `Compiler::register_native_module`
  in `compiler/src/compiler.rs:56` already accepts a module name and
  a `ModuleExports`. After this work, that method either goes away
  (if all stdlib is `.pa`) or stays only for the `math.pi`-style
  constants.

## What we considered

### Type-annotation syntax: greedy app in fun params (rejected)

An earlier draft of the syntax (proposed by the user, then revised)
treated function-parameter position as **greedy generic application**:
`fun list a -> int` = one arg of type `list<a>`; for multi-arg, every
param parenthesized: `fun (list a) (list b) -> bool`.

We dropped this in favor of the current convention (Rule A — atom =
arg, parens around generic-applied) for three reasons:

1. **The parser already supports Rule A.** `parse_type_fun` reads
   each atom as one param via the non-greedy
   `parse_type_expression`. Zero parser changes.
2. **It's already documented and used.** Trait method sigs in
   `trait-numeric-scaffold/main.pa` use `fun a a -> a` for a binary
   function. Switching conventions would force rewriting these.
3. **One uniform rule.** Greedy app lives in single-type contexts;
   atom-separation lives in multi-arg contexts. Both rules exist
   today; we're not adding a third grammar.

Cost: `fun (list a) -> int` instead of `fun list a -> int` — one
pair of parens per generic-applied param. Acceptable.

### A separate `extern def` form (rejected)

Considered: `extern def length :: fun (list a) -> int = "list-length"`
as a top-level declaration, separate from regular `def`. Rejected
because (a) it requires a new keyword, (b) it splits the def
namespace into two top-level forms with subtly different shapes, and
(c) `built-in "tag"` as a regular expression integrates more cleanly
with how the analyzer already treats RHS expressions.

### Tag-string-as-type registry (rejected)

Considered: `def length = built-in "list-length"` with no annotation
— the type comes from a Rust-side `tag → Type` registry. Rejected
because it defeats half the readability goal (types disappear from
the `.pa` file) and forces the parser to special-case `built-in` for
type inference.

## Implementation phases

### Phase 1 — Top-level `def` annotations

Goal: `def name :: <type> = <expr>` parses, the body's inferred type
unifies with the annotation, and mismatches are reported as
diagnostics. No stdlib changes yet.

Steps:

1. **AST.** Add `type_annotation: Option<TypeExprNode>` to the
   value-def shape in `compiler/src/ast/definition.rs`. Threads
   through the `DefinitionKind::Expr` variant.
2. **Parser.** In `parse_definition` (around the value-def path in
   `compiler/src/parser.rs`), after parsing the name and before
   expecting `=`, peek for `Token::DoubleColon`. If present, consume
   and call `parse_type_expression_with_generics`. Attach to the
   AST node.
3. **Analyzer.** In the value-def constraint-gen path
   (`compiler/src/analyzer.rs`), if an annotation is present,
   resolve it via `type_expr_to_type` and emit a constraint:
   `inferred_body_ty ~ annotated_ty`. The annotated type also seeds
   the binding's `Scheme` so cross-module imports use the annotation
   verbatim (not the inferred type — they should match, but the
   annotation is the contract).
4. **Tests.** Add fixtures in `tests/analyze/`:
    - `def-annotation-int` — `def x :: int = 0`.
    - `def-annotation-fun` — `def add :: fun int int -> int = fun x y { x + y }`.
    - `def-annotation-polymorphic` — `def id :: fun a -> a = fun x { x }`.
    - `def-annotation-mismatch` — `def x :: int = "hello"` should diagnose.
    - `def-annotation-generic-applied` — `def first :: fun (list a) -> option a = fun xs { ... }`.

`where` constraints on the annotation can wait — none of the stdlib
modules we'd migrate first (`core.list`, `core.string`, `core.math`)
use them. `core.map` does, so it lands later.

### Phase 2 — The `built-in "tag"` expression

Goal: `built-in "list-length"` parses and analyzes as a value of the
enclosing def's annotated type; codegen emits the builtin value.

Steps:

1. **Lexer.** `built-in` becomes a contextual keyword (or a regular
   keyword if there's no conflict — `built-in` with the hyphen
   probably tokenizes as an identifier today; check before deciding).
2. **AST.** Add `ExprKind::Builtin(String)` (the tag) in
   `compiler/src/ast/expr.rs`.
3. **Parser.** Recognize `built-in "<string-literal>"` as an
   expression. Reject if the literal isn't a plain string (no
   interpolation, no escapes that materially change content).
4. **Analyzer.** When constraining a `built-in` expression: require
   the enclosing context to be the immediate RHS of a type-annotated
   top-level `def`; otherwise emit a diagnostic. The expression's
   type is the annotation. (Implementation note: the simplest way is
   to handle `built-in` specially in the value-def path, not in the
   general `constrain_expr` recursion.)
5. **Codegen** (`codegen/src/emit.rs`). At global-init for a def
   whose RHS is `Builtin(tag)`, look up `tag` in the tag table and
   emit `Value::Builtin(builtin)` directly into the global slot.
   Unknown tag → diagnostic.
6. **Tests.** A `run/` fixture using a `built-in`-defined value end
   to end; an `analyze/` fixture for the unknown-tag case.

### Phase 3 — Stdlib `.pa` loader

Goal: `use core.list` (and friends) loads from a baked-in `.pa`
source rather than going through `register_native_module`.

Steps:

1. **Source registry.** Add `compiler/src/stdlib/` containing
   `list.pa`, `string.pa`, etc. Export a
   `pub fn stdlib_sources() -> &'static [(&'static str, &'static str)]`
   that returns `("core.list", include_str!("stdlib/list.pa"))` for
   each.
2. **Loader.** In `Compiler::load_module`, before doing filesystem
   lookup, check the stdlib registry for the requested module name.
   If found, parse + analyze the baked source (same path as
   `load_prelude`).
3. **Tag table.** Add `vm::stdlib::builtin_table()` returning
   `&'static [(&'static str, Builtin)]`. Codegen consults it from
   Phase 2's `Builtin(tag)` lowering.
4. **Sunset `register_native_module` (partial).** Keep it for
   `math.pi`/`math.e` constants. Remove the per-def listings — they
   live in `.pa` now.

### Phase 4 — Per-module migration

One `.pa` file per current `NativeModule`. Pilot order:

1. `core.list` — many defs, several pure-Pluma candidates
   (`is-empty`), exercises generics. Good first target.
2. `core.string` — simple signatures, no `where`. Confirms the
   bytes/result interop reads cleanly.
3. `core.math` — has constants (`pi`, `e`). Forces the decision on
   the constants question (built-in form vs registered constant).
4. `core.io`, `core.bytes`, `core.ref`, `core.regex`, `core.option`,
   `core.result` — straightforward translations.
5. `core.map` — needs `where (hash k)` on value annotations. Lands
   last because it's the only module that exercises the constraint.

Each phase ends with: the old `NativeModule` removed from
`vm/src/stdlib.rs`, the new `.pa` file in place, snapshots updated.

## Open questions

- **Math constants.** Do `math.pi` and `math.e` get a
  `built-in-constant "tag"` form, or stay as
  `register_native_module`-injected values? Decide during Phase 3.
- **Visibility of `built-in`.** Should the `built-in` keyword be
  parseable in user `.pa` code at all, or restricted to stdlib
  sources? Restricting feels right (it's an implementation primitive,
  not a user-facing feature), but enforcing requires a "this module
  is a stdlib module" flag on `Compiler`. Easy to add later if we
  want it.
- **Doc-comment exposure.** Where do `# ...` comments above defs end
  up in the typed AST so the LSP can serve them as hover-docs? Today
  `Module` has a comment map but nothing connects a comment to its
  following def. This work might be the right time to wire that up,
  or it could be its own follow-up.
