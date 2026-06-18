# Language reference

The reference describes Pluma as it is: the syntax, the type system, and the
standard library. It's organized so you can read it front to back the first
time, then jump back to any single section later.

The reference is split into focused pages — listed here and in the sidebar.

## Reference pages

- **[Operators](/docs/reference/operators)** — arithmetic, comparison, logical,
  bitwise, coalesce, and pipe, with their signatures and precedence.
- **[Type aliases](/docs/reference/aliases)** — naming types, and when two types
  count as the same (nominal enums, structural records).
- **[Bytes](/docs/reference/bytes)** — binary data, byte literals, and the
  `std/bytes` module.
- **[Regular expressions](/docs/reference/regex)** — the structured regex DSL:
  atoms, classes, quantifiers, anchors, and `std/regex`.
- **[Diagnostics](/docs/reference/diagnostics)** — the stable error and lint
  codes the compiler emits.
- **[Fullstack build](/docs/reference/build)** — what `pluma build` produces and
  how a page reaches the screen (SSR vs CSR).

## The standard library

Every `std/` module is documented from its source at the
[stdlib reference](/std/list).
