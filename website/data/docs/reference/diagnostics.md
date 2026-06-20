# Diagnostics and error codes

When the compiler rejects a program it emits one or more *diagnostics*. Each
carries a *stable code*: it never changes meaning across releases, so it's safe
to link to, filter on, or cite.

```
error[E0100]: Name `lenght` is not defined.
  ╭─▸ src/main.pa:3:12
  │
3 │ def main = lenght
  │            ^^^^^^
  │
  ╰─ help: did you mean `length`?
```

The severity (`error` or `warning`) leads, then the code, then the primary
location and the caret span. A `help:` line is a single actionable suggestion
(misspelled names, fields, and variants get a `did you mean ...?` hint); a `note:`
adds context. The language server surfaces all of it inline.

## Parse errors

| Code | Meaning |
| --- | --- |
| `E0001` | Empty regular expression |
| `E0002` | Empty grouping in a regular expression |
| `E0003` | Empty repetition count in a regular expression |
| `E0008` | Invalid regular-expression count modifier |
| `E0009` | Quantifier applied to a regex anchor |
| `E0010` | Invalid expression after `.` (expected a field name or tuple index) |
| `E0011` | Invalid `def` body (expected an expression or a type) |
| `E0012` | Missing return type after `->` |
| `E0013` | Overflowing integer literal |
| `E0014` | Invalid duration literal |
| `E0015` | Duration units out of order |
| `E0017` | Unclosed string interpolation |
| `E0018` | Unclosed string |
| `E0020` | Invalid \x escape in a bytes literal |
| `E0022` | Expected an expression after `...` |
| `E0023` | Expected an expression after `defer` |
| `E0024` | Misplaced record spread (`...` must come first, once) |
| `E0025` | Unexpected end of file |
| `E0026` | Unexpected token |
| `E0027` | Unexpected token at the top level |
| `E0028` | Misplaced `public` / `opaque` |
| `E0029` | Expected an expression |

## Analysis and type errors

| Code | Meaning |
| --- | --- |
| `E0100` | Name is not defined |
| `E0101` | Name is never used (warning) |
| `E0102` | Type mismatch |
| `E0103` | Failed to unify a recursive type |
| `E0104` | Parameter count mismatch |
| `E0105` | Tuple size mismatch |
| `E0106` | Tuple index out of range |
| `E0107` | Record field does not exist |
| `E0108` | Enum variant does not exist |
| `E0109` | Non-exhaustive `when` |
| `E0111` | Ambiguous bare trait method |
| `E0112` | Duplicate top-level definition |
| `E0113` | No trait instance for a type |
| `E0114` | Type can't cross the wire |
| `E0116` | Incomplete trait instance (missing method) |
| `E0118` | Overlapping instance |
| `E0119` | Orphan instance |
| `E0120` | Refutable pattern in a `let` |
| `E0122` | `try` right-hand side has an undetermined type |
| `E0123` | `try` on an unsupported carrier |
| `E0132` | Item is private to its module |
| `E0133` | `remote def` (RPC endpoint) is not `public` |
| `E0134` | `remote def` has an invalid endpoint signature |
| `E0135` | Bare variant must be qualified by its enum |

## Lints

Lints are advisory warnings from `pluma lint`: stylistic and correctness smells
the type-checker tolerates. They never stop a run, build, or test, but
`pluma lint` exits non-zero if any fire, so it can gate CI.

| Code | Meaning |
| --- | --- |
| `L0001` | `let _ = expr` binds nothing; drop the `let _ =` |
| `L0002` | `try _ = expr` binds nothing; write just `try expr` |
| `L0003` | Comparing to a boolean literal (`x == true`) is redundant |
| `L0004` | `if c { true } else { false }` is just the condition |
| `L0005` | Boolean-literal operand of `and` / `or` is redundant |
| `L0006` | Function only forwards its arguments (`fun x { f x }` is `f`) |
| `L0007` | Both branches of an `if` are identical |
| `L0008` | A binding returned immediately doesn't need the `let` |
| `L0009` | Repeated `ns.member` projections read better as a `using ns` block |
| `L0010` | Inside `using ns`, the `ns.` prefix is redundant; write `.member` |
