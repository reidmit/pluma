# Errors and missing values

Most languages have a few different stories for "this might not have a value"
and "this might fail": null, exceptions, error codes, sometimes all three. Pluma
has one idea, used two ways. A value that might be absent is an `option`. A
value that might fail with an explanation is a `result`. Both are ordinary
enums, so the compiler makes you account for the empty case, and a failure is a
value you pass around like any other, not a surprise that escapes up the stack.

This page covers both, the two shortcuts (`??` and `try`) that make them
pleasant to work with, and `std/error`, the carrier an app reaches for once it
stops caring exactly which failure it got.

## option: a value that might be absent

An `option` is either `some x`, holding a value, or `none`, holding nothing.
Pluma builds it in, so you never write `null` and never get a surprise empty
value: the type says it might be missing, and the compiler makes you handle it.

```pluma
def first = fun xs {
	when xs is [head, ...] {
		some head
	} is [] {
		none
	}
}

first [9, 8, 7]   # => some 9
first []          # => none
```

`some` and `none` are written bare (no module prefix) because `option` is one
of the two enums seeded into every file. (Every other enum reaches its variants
through its name, like `color.red`; `option` and `result` are the exception.)

## result: a value that might fail

A `result` is either `ok x`, holding a success, or `err e`, holding an
explanation of what went wrong. Use it whenever a function can fail in a way the
caller should hear about.

```pluma
def divide = fun a b {
	if b == 0 {
		err "can't divide by zero"
	} else {
		ok (a / b)
	}
}

divide 10 2   # => ok 5
divide 10 0   # => err "can't divide by zero"
```

The failure here is a `string`, which is the common choice, but `err` can carry
any type. A precise, closed enum is often better, since `when` can then check
that you handled every failure it defines:

```pluma
enum parse-error {
	empty-input
	bad-digit
}

def parse-byte :: fun string -> result int parse-error = fun s {
	# ... returns ok n, or err parse-error.empty-input, etc.
}
```

The type `result int parse-error` reads left to right: a result whose success is
an `int` and whose failure is a `parse-error`.

## Handling them: when

Because both are ordinary enums, the always-available tool is `when`, which
makes you cover every case:

```pluma
when divide 10 0 is ok q {
	print "got $(to-string q)"
} is err message {
	print "failed: $(message)"
}
```

That's exhaustive and explicit, but writing it at every step gets noisy. Two
shortcuts handle the common shapes.

## ??: supply a fallback

The coalesce operator `??` unwraps a `some` or an `ok`, and falls back to the
value on its right when it meets a `none` or an `err`:

```pluma
(first [9, 8]) ?? 0   # => 9
(first []) ?? 0       # => 0
(divide 10 0) ?? -1   # => -1
```

It's lazy in its right operand (the fallback is only evaluated if it's actually
needed) and right-associative, so you can chain fallbacks and the first
non-empty one wins:

```pluma
(env "PORT") ?? (env "DEFAULT_PORT") ?? "8080"
```

## try: unwrap, or bail out early

`??` is for "I have a sensible default." When you don't, when a failure should
abandon the rest of the work and hand the problem back to your caller, reach for
`try`. It unwraps a success and binds it to a name; on a failure, it stops the
current function and returns that failure outward.

```pluma
def run = fun a b {
	try q = divide a b   # bind q to the success, or return the err now
	ok (q + 1)
}

run 10 2   # => ok 6
run 10 0   # => err "can't divide by zero"
```

A chain of fallible steps then reads top to bottom, with the error path implied
rather than spelled out at every line:

```pluma
def load-config = fun path {
	try text = read-file path
	try parsed = parse text
	try port = field parsed "port"
	ok port
}
```

Each `try` either continues with the unwrapped value or short-circuits the whole
function with the first failure it hits. The same `try` works on an `option`
(short-circuiting with `none`) and, as the [async](/docs/guides/server) pages
cover, on a `task`. One rule: a single function's `try` chain sticks to one of
these: mix an `option` step into a `result` chain and the compiler will flag it.

::: aside .callout
**`??` recovers, `try` propagates.** They're duals. Use `??` at the spot where
you can carry on with a default; use `try` where the only sensible move is to
let the failure travel up to someone who can deal with it.
:::

## The helper modules

`std/option` and `std/result` round out the two types with the transformations
you'd otherwise write by hand. A few you'll reach for often:

| Call | What it does |
| --- | --- |
| `option.map o f` | Apply `f` to the value inside a `some`, leave `none` alone |
| `option.then o f` | Chain a step that itself returns an `option` |
| `option.is-some o` | True when there's a value |
| `result.map r f` | Transform an `ok` value, leave an `err` alone |
| `result.map-err r f` | Transform the `err`, leave an `ok` alone |
| `result.to-option r` | Drop the explanation: `ok x` becomes `some x`, any `err` becomes `none` |
| `option.to-result o e` | Add an explanation: `some x` becomes `ok x`, `none` becomes `err e` |

```pluma
use std/option

option.map (first [9, 8]) (fun n { n * 10 })   # => some 90
option.map (first []) (fun n { n * 10 })        # => none
```

`map` and `then` are the workhorses: `map` transforms the value when it's there
and quietly passes an empty/failed value through, and `then` is `map` for a step
that might itself come up empty, so the result doesn't end up doubly wrapped.

## std/error: erasing late

A precise failure type is exactly what you want at the leaf, where a caller might
react to a specific case. But as a fallible chain crosses layers, the precise
type stops paying for itself: at some point you only want to propagate the
failure and, eventually, report it. Threading a different error enum through
every layer becomes friction with no payoff.

`std/error` is the carrier for that moment. Declare a function's failure type as
`error`, and `try` will *erase* each precise failure into it as it propagates,
with no manual conversion:

```pluma
use std/error

def render :: fun int -> result string error = fun id {
	try user = load-user id     # load-user returns result _ db-error
	ok "<page>$(user)</page>"
}
# on failure, error.message reads like "timed out"
```

The idea is to **erase late**: keep your signature precise (`result _ db-error`)
for as long as someone matches on it, and switch to `error` only at the layer
that has stopped caring which failure it was. Libraries stay precise so their
callers can react; apps erase at the seam where they just want to log and move
on.

### A trace that survives async

An `error` isn't just a message: it's a chain of frames, each a message with an
optional cause beneath it. `error.context` adds a layer, putting new information
on top and the original underneath, so the chain reads like a stack trace you
built on purpose:

```pluma
use std/error

# message => "loading user 7: connection refused"
error.context "loading user 7" (error.new "connection refused")
```

Read the chain back as one line with `error.message`, or as its separate levels
with `error.trace`:

```pluma
error.message err   # => "loading user 7: querying db: timed out"
error.trace err     # => ["loading user 7", "querying db", "timed out"]
```

Because the chain is plain data, not a native stack trace, it survives the
boundaries an async program crosses: the trace through a `task` reads the same
as one through a plain `result`.

### Opting in, and recovering the leaf

Erasing is deliberate, not automatic: a type erases into `error` only if it
knows how to describe itself for a human. You grant that with a `describe`
instance:

```pluma
use std/error

implement describe db-error {
	def describe = fun e {
		when e is db-error.timeout {
			"timed out"
		} is db-error.no-such-row {
			"no such row"
		}
	}
}
```

After that, every `try` that propagates a `db-error` into an `error`-typed
function erases it for free. `string` already has a `describe` instance, so every
standard API that fails with a string (`fs`, `sql`, `http`, …) erases the moment
you call it from an `error` context, no setup required.

Erasing loses the precise type, but not the ability to branch on it. `error.is-a`
asks whether a failure ultimately came from a particular constructor, however
many layers of context sit on top:

```pluma
if error.is-a err db-error.timeout {
	retry ()
} else {
	give-up ()
}
```

So an app can still retry a timeout or 404 a missing row while carrying the
single `error` type everywhere above the leaf. The precise enum stays the source
of truth; `error` is the convenience for the layers that have stopped looking.

## Where to go next

- **[Operators](/docs/reference/operators)**: the full precedence and signature
  table for `??`, comparison, and the rest.
- **[Type aliases and nominal types](/docs/reference/aliases)**: how enums like
  `option` get their identity, and why records work differently.
- **[Diagnostics](/docs/reference/diagnostics)**: the error codes the compiler
  emits when a failure case goes unhandled.
