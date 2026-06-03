+++
title = "Control flow"
description = "`if`, `when`, and `while` all use patterns. They're expressions — every form has a value."
weight = 7
+++

## `if` expressions

Single-armed pattern matching with an optional `else` arm. Not limited to booleans! For multiple cases, use `when`.

Without `else`, an `if` evaluates to `nothing`; with `else`, it evaluates to the common type of both branches.

```pluma
if some-value is 47 {
    print "ok cool"
}

if some-animal is dog name {
    print "it's a dog called $(name)"
}

# `else` runs when the pattern doesn't match
if result is ok value {
    print "success! got $(value)"
} else {
    print "something went wrong"
}

# used as a value
let label = if n is some v { "got $(to-string v)" } else { "none" }
```

The `is PATTERN` is optional. When omitted, the subject is matched against `true`, so a plain boolean condition reads naturally — `if cond { ... }` is exactly `if cond is true { ... }`:

```pluma
if n > 10 {
    print "big"
} else if n > 0 {
    print "small"
} else {
    print "non-positive"
}
```

Because both record literals and blocks are written with `{ ... }`, a `{` in the subject always opens the body. If the subject passes a record literal, parenthesize that argument so its brace isn't read as the body:

```pluma
if enabled ({ verbose: true }) {
    print "on"
}
```

## `when` expressions

Must be exhaustive — all cases must be covered. `else` is the catch-all branch (equivalent to `is _`); use whichever reads better. Evaluates to the value of the first matching case; all cases must have the same type.

```pluma
when some-value is 47 {
    print "ok cool"
} else {
    print "it's something else"
}

when result is ok value {
    print "success! got $(value)"
} is error message {
    print "failed: $(message)"
}
```

Exhaustiveness is checked structurally for `bool` and enum subjects; other subject types currently rely on an `else` or catch-all.

## `while` expressions

Pattern-matching loop. Runs the body as long as the subject matches. As with `if`, the `is PATTERN` is optional and defaults to `is true`, so a boolean loop condition is written bare:

```pluma
while keep-going {
    print "ya"
}

let iterator = iterate names
while (get-next iterator) is some name {
    print "name: $(name)"
}
```

## `defer`

`defer expr` schedules `expr` to run when the enclosing **function** exits — by any path: a normal return, or a `try` that short-circuits on failure. It's the tool for cleanup that must happen no matter how the function ends.

```pluma
def read-config = fun path {
    let f = io.open path
    defer io.close f       # runs on every exit below
    try contents = io.read-all f
    parse contents
}
```

Even if `io.read-all` fails and the `try` propagates the error, `f` is still closed.

Multiple `defer`s run last-in-first-out, so acquisition and release nest correctly across unrelated resources:

```pluma
def diff-files = fun a b {
    let fa = io.open a
    defer io.close fa
    let fb = io.open b
    defer io.close fb       # fb closes first, then fa
    try xa = io.read-all fa
    try xb = io.read-all fb
    ok (compute-diff xa xb)
}
```

A `defer` only fires if execution actually reached it: one guarded by an `if` runs only when that branch ran, and a `defer` written after a `try` is skipped when that `try` short-circuits. The deferred expression's value is discarded — `defer` itself evaluates to `nothing`.

{% note() %}
See [Patterns](@/docs/patterns.md) for the full grammar of patterns usable in `is` arms.
{% end %}
