# pencil

## phase 1 (done)

no generics, no mutation, no let-binding-patterns

basic types (int, float, string, bool, regex), tuples, records, functions, string interpolation, top-level `def`s, alias types, enum types (nominal, with payload variants and recursion).

## phase 1.25 (done)

match expressions w/ patterns

`if` / `when` / `while` pattern matching: literal, identifier, wildcard, constructor (multi-arg + nested via parens), tuple, record patterns. `when` exhaustiveness-checked.

## phase 1.5 (done)

imports of other modules (`use`)

`use a.b.module` resolves dotted paths against the root dir. `use ... as alias` for local renaming. Cross-module access for values, enums, and aliases via `module.name` (and `module.enum.variant` for variants), with per-use polymorphism. Cycle detection.

## phase 2

- generics?
- **numeric polymorphism.** `+`, `-`, `*`, `/`, `%`, `<`, `<=`, `>`, `>=` dispatch on the operand types (Int if unknown, Float if either operand is concretely Float). That works for direct uses like `1.5 + 2.5`, but `fun a b { a + b }` resolves to `int -> int -> int` because both params start as fresh type vars and the default kicks in. A genuine polymorphic-numeric function would need a type-class or trait-like constraint ("a must be Numeric") that the type system doesn't have. For now, users write a per-type function (`add-int`, `add-float`) if they need both.

## phase 3

- mutability?

## phase 4

- destructuring patterns in let bindings?
- destructuring patterns in lambda args?
