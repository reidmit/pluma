# pencil

## phase 1 (done)

no generics, no mutation, no let-binding-patterns

basic types (int, float, string, bool, regex), tuples, records, functions, string interpolation, top-level `def`s, alias types, enum types (nominal, with payload variants and recursion).

## phase 1.25 (done)

match expressions w/ patterns

`if` / `when` / `while` pattern matching: literal, identifier, wildcard, constructor (multi-arg + nested via parens), tuple, record patterns. `when` exhaustiveness-checked.

## phase 1.5 (done)

imports of other modules (`use`)

`use a.b.module` resolves dotted paths against the root dir. `use ... as alias` for local renaming. Cross-module value access via `module.name` with per-use polymorphism. Cycle detection.

## phase 2

- generics?

## phase 3

- mutability?

## phase 4

- destructuring patterns in let bindings?
- destructuring patterns in lambda args?
