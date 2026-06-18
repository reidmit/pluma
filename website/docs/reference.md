# Language reference

The reference describes Pluma as it is: the syntax, the type system, and the
standard library. These pages are for looking things up — precedence tables,
error codes, the exact shape of a feature.

If you're meeting the language for the first time, start with the
[Language tour](/docs/tour/basics) instead: it builds Pluma up from the ground,
one idea at a time, and reads front to back. Come back here for the details.

The reference is split into focused pages — listed here and in the sidebar.

## Reference pages

- **[Operators](/docs/reference/operators)** — arithmetic, comparison, logical,
  bitwise, coalesce, and pipe, with their signatures and precedence.
- **[Errors and missing values](/docs/reference/errors)** — `option`, `result`,
  the `??` and `try` shortcuts, and erasing late with `std/error`.
- **[Concurrency](/docs/reference/concurrency)** — tasks, awaiting with `try`,
  running work concurrently, `scope`, and `defer`.
- **[Type aliases](/docs/reference/aliases)** — naming types, and when two types
  count as the same (nominal enums, structural records).
- **[Using blocks](/docs/reference/using)** — the `using M { ... }` ambient-module
  syntax and its leading-dot shorthand.
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
