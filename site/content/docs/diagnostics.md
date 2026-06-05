+++
title = "Diagnostics"
description = "How Pluma reports parse and type errors, and the stable code for every one."
weight = 11
+++

When the compiler rejects a program it emits one or more **diagnostics**. Every
diagnostic the frontend produces (parse errors from the tokenizer/parser, and
analysis/type errors from the checker) carries a **stable code** so you, your
editor, and tooling can refer to it precisely.

## Anatomy of a diagnostic

```
error[E0100]: Name `lenght` is not defined.
  ╭─▸ src/main.pa:3:12
  │
3 │ def main = lenght
  │            ^^^^^^
  │
  ╰─ help: did you mean `length`?
```

The left margin is a single unbroken rail: it opens at the location (`╭─▸`), runs
down the source excerpt (`│`), and closes through the help/notes (`╰─`).

- **`error` / `warning`** — the severity. Warnings (e.g. an unused binding) don't
  stop compilation on their own.
- **`[E0100]`** — the stable code. It never changes meaning across releases, so
  it's safe to link to, filter on, or cite.
- **`╭─▸ file:line:col`** — the primary location (1-based).
- **The caret span** points at the offending source.
- **`help:`** — a single actionable suggestion. Misspelled names, fields, and
  enum variants get a `did you mean ...?` hint computed from what's actually in
  scope.
- **`note:`** — extra context (e.g. the fields a record actually has).

Some diagnostics carry a **secondary label** — a second pointed-at location. For
example, a duplicate definition points at both the redefinition and the original:

```
error[E0112]: Duplicate top-level definition `config`.
  ╭─▸ src/main.pa:3:5
  │
1 │ def config = 1
  │     ^^^^^^ previous definition here
  ┆
3 │ def config = 2
  │     ^^^^^^
  ╰─
```

(The dashed `┆` segment stands in for source lines skipped between the two spans.)

The language server surfaces all of this too: the code appears on the diagnostic,
help/notes are folded into the hover message, and secondary labels become related
information.

## Parse errors (`E00xx`)

| Code | Meaning |
|------|---------|
| E0001 | Empty regular expression |
| E0002 | Empty grouping in a regular expression |
| E0003 | Empty repetition count in a regular expression |
| E0004 | Invalid binary digits |
| E0005 | Invalid (decimal) digits |
| E0006 | Invalid hex digits |
| E0007 | Invalid octal digits |
| E0008 | Invalid regular-expression count modifier |
| E0009 | Quantifier applied to a regex anchor (`^`, `$`, `%`) |
| E0010 | Invalid expression after `.` (expected a field name or tuple index) |
| E0011 | Invalid `def` body (expected an expression or a type) |
| E0012 | Missing return type after `->` |
| E0013 | Overflowing integer literal |
| E0014 | Invalid duration literal |
| E0015 | Duration units out of order |
| E0016 | Overflowing duration literal |
| E0017 | Unclosed string interpolation |
| E0018 | Unclosed string |
| E0019 | Invalid escape in a bytes literal |
| E0020 | Invalid `\x` escape in a bytes literal |
| E0021 | `built-in` requires a plain string literal tag |
| E0022 | Expected an expression after `...` |
| E0023 | Expected an expression after `defer` |
| E0024 | Misplaced record spread (`...` must come first, once) |
| E0025 | Unexpected end of file |
| E0026 | Unexpected token |
| E0027 | Unexpected token at the top level |
| E0028 | Misplaced `public` / `opaque` |
| E0029 | Expected an expression (e.g. a missing def body or operator operand) |

## Analysis & type errors (`E01xx`)

| Code | Meaning |
|------|---------|
| E0100 | Name is not defined |
| E0101 | Name is never used *(warning)* |
| E0102 | Type mismatch |
| E0103 | Failed to unify a recursive type |
| E0104 | Parameter count mismatch |
| E0105 | Tuple size mismatch |
| E0106 | Tuple index out of range |
| E0107 | Record field does not exist |
| E0108 | Enum variant does not exist |
| E0109 | Non-exhaustive `when` |
| E0110 | Ambiguous variant |
| E0111 | Ambiguous bare trait method |
| E0112 | Duplicate top-level definition |
| E0113 | No trait instance for a type |
| E0114 | Type can't cross the wire |
| E0115 | Unsupported instance head |
| E0116 | Incomplete trait instance (missing method) |
| E0117 | Ambiguous trait-method dispatch (unbound type variables) |
| E0118 | Overlapping instance |
| E0119 | Orphan instance |
| E0120 | Refutable pattern in a `let` |
| E0121 | Duplicate field in a record pattern |
| E0122 | `try` right-hand side has an undetermined type |
| E0123 | `try` on an unsupported carrier |
| E0124 | `try` with no continuation |
| E0125 | `try` with an unsupported left-hand pattern |
| E0126 | `??` left-hand side has an undetermined type |
| E0127 | `??` on an unsupported carrier |
| E0128 | `built-in` requires a type annotation |
| E0129 | `built-in` must be a top-level def's right-hand side |
| E0130 | Unknown regex character class |
| E0131 | `where`-clause type variable not in the signature |
| E0132 | Item is private to its module |
| E0133 | `remote def` (RPC endpoint) is not `public` |
| E0134 | `remote def` has an invalid endpoint signature |
| E0135 | Bare variant must be qualified by its enum |

The authoritative source for these codes is the `code()` method on `ParseError`
and `AnalysisError` in the `compiler` crate; this table mirrors it. The
`tests/errors/` snapshot suite exercises one fixture per error path and pins the
exact rendered output, so the messages here stay honest.
