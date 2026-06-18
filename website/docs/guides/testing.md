# Testing

Pluma's testing story has a twist worth stating up front: **tests are a library,
not syntax.** There's no `@test` annotation or special `test` keyword. A test
suite is an ordinary value — a list — that you build with `std/test` and check
with `std/assert`, and the `pluma test` command finds and runs it. That means you
can build, group, and transform tests with the same tools you use for any other
data.

## A test file

A test file is named `*.test.pa` and exports a single definition called `tests`,
typed `test.suite`. Each entry is a `test.case` with a name and a body:

```pluma
use std/test
use std/assert

def tests :: test.suite = [
	test.case "adds" (fun { assert.equals (2 + 3) 5 }),
	test.case "compares" (fun { assert.is-true (2 < 3) }),
]
```

A case's body is a function that returns a [`result`](/docs/reference/errors):
`ok ()` means the test passed, and `err message` means it failed. You rarely
write those by hand, though — the `std/assert` checks produce exactly that result,
so a case body is usually just one assertion.

## Assertions

`std/assert` covers the common checks, each returning the pass/fail `result` a
case body wants:

| Check | Passes when |
| --- | --- |
| `assert.equals a b` | `a` equals `b` (compared by value) |
| `assert.not-equals a b` | `a` differs from `b` |
| `assert.is-true c` / `assert.is-false c` | the bool is `true` / `false` |
| `assert.is-some o` / `assert.is-none o` | the option is `some` / `none` |
| `assert.is-ok r` / `assert.is-err r` | the result is `ok` / `err` |

When one case needs several checks, `assert.all` runs a list of them and passes
only if every one does:

```pluma
test.case "the basics hold" (fun {
	assert.all [
		assert.is-true (1 < 2),
		assert.is-some (some 7),
		assert.equals (2 + 2) 4,
	]
})
```

## Organizing a suite

Because a suite is just a list, you shape it with a few constructors:

- `test.group "name" [...]` nests related cases under a heading.
- `test.skip case` keeps a case in the file but doesn't run it.
- `test.focus case` runs *only* the focused cases — handy while chasing one
  failure.
- `test.todo "name"` records a test you intend to write, with no body yet.

```pluma
use std/test
use std/assert

def tests :: test.suite = [
	test.case "adds" (fun { assert.equals (2 + 3) 5 }),
	test.group "edge cases" [
		test.case "zero" (fun { assert.equals (0 + 0) 0 }),
	],
	test.todo "handles overflow",
]
```

## Running them

`pluma test` discovers every `*.test.pa` file in your project, runs the cases, and
reports the results. Tests run under V8 — the same engine your built artifact
deploys to — so a passing test exercises the exact code that ships.

```
pluma test
```

The output mirrors the structure of your suite, with a tick for each pass, a
cross for each failure (and the assertion's message), and a summary at the end:

```
demo
  ✓ adds
  edge cases
    ✓ zero
    ✓ multiple checks
  ○ handles overflow (todo)
  ✗ deliberate failure
      expected 5, got 4

3 of 4 passed, 1 todo
```

A failed assertion fails its case with a readable message — `assert.equals`, for
instance, reports what it expected against what it got — so you can usually see
what went wrong without opening the file.

## See also

- **[Command-line script](/docs/guides/cli)** — the broader `pluma` toolbelt,
  including `run`, `build`, and `format`.
- **[Errors and missing values](/docs/reference/errors)** — the `result` type a
  test case returns.
