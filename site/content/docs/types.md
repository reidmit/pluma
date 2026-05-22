+++
title = "Types"
description = "Aliases and enums. Records are structural; enums are nominal."
weight = 8
+++

## Alias types

An `alias` introduces a named type. Two forms:

```
alias person {
    name :: string
    age  :: int
}

alias number-list list int
```

The first form is a record-type alias (fields use `::`). The second is a bare type-expression alias.

## Enum types

Enums are **nominal**: two enums with the same shape are distinct, and references within an enum's body (e.g., `tree` inside `tree`'s `node` variant) are allowed.

```
enum color {
    red
    green
    blue
}

enum tree {
    empty
    node int tree tree
}

enum bool {
    true
    false
}
```

Variants are accessed by qualifying with the enum name. Zero-arg variants are values of the enum type; payload variants are constructor functions.

```
let c = color.red                          # c : color
let t = tree.node 1 tree.empty tree.empty
```

Bare variant names also work when unambiguous (`red` instead of `color.red`). If two enums in scope share a variant name, the local-module enum wins; if both are non-local, you get an `AmbiguousVariant` error and need to qualify.

### Generic enums

Enums can take type parameters, listed space-separated after the name. Variants reference them by name.

```
enum option a {
    some a
    none
}

enum result a b {
    ok a
    err b
}

enum pair a b {
    both a b
    left a
    right b
}
```

Instantiate with space-separated type arguments in any type position:

```
alias maybe-int option int

alias named-list {
    name  :: string
    items :: list (option int)
}
```

Multi-arg type contexts (variant params) are non-greedy — wrap generic applications in parens there:

```
enum container a { holds (option a) }
```

### Prelude enums

`option` and `result` are seeded into every module. No `use` needed; their variants (`some`, `none`, `ok`, `err`) work bare:

```
let n = some 5             # n : option int
let nothing = none         # nothing : option a
let outcome = ok 42        # outcome : result int b
let oops = err "boom"      # oops : result a string

when outcome is ok v {
    print v
} is err msg {
    print msg
}
```

## Nominal identity

Enums carry a fully-qualified name internally (`<defining-module>.<enum-name>`). Two enums with the same fields but different modules or names are distinct types and won't unify. In error messages and snapshots, only the bare name is shown.

## Records are structural

Records are **row-polymorphic** and structural — there's no nominal record type. A function that destructures a few fields stays generic over the others:

```
# `fun p { p.name }` is typed `{name: a, ...} -> a`
def name-of = fun p { p.name }
```

To name a record shape, use an `alias`:

```
alias point {
    x :: float
    y :: float
}
```
