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

Pattern-matching loop. Runs the body as long as the subject matches.

```pluma
while some-value is true {
    print "ya"
}

let iterator = iterate names
while (get-next iterator) is some name {
    print "name: $(name)"
}
```

{% note() %}
See [Patterns](@/docs/patterns.md) for the full grammar of patterns usable in `is` arms.
{% end %}
