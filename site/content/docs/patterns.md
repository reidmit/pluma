+++
title = "Patterns"
description = "Patterns appear in `if`, `when`, `while`, and `let`. `let` only accepts irrefutable patterns."
weight = 5
+++

## Refutability

An **irrefutable** pattern always matches: identifiers, wildcards, tuples, records, and nestings of those. A **refutable** pattern can fail to match: constructor patterns, literals, list patterns with required elements, string-interpolation patterns. Refutable patterns must appear inside `if`/`when`/`while`, never `let`.

## Record patterns

Record patterns destructure records. By default they require an **exact** match on the field set; add `, ...` to allow extras.

```
let {name: n, age: a} = {name: "reid", age: 28}   # exact: types must match
let {name: n, ...} = {name: "alice", age: 30}     # open: extras ignored

when person is {name: n, role: r} { ... }         # exact `{name, role}` only
when person is {name: n, ...} { ... }             # any record with a `name`
```

| Pattern | Matches |
| - | - |
| `{a: x, b: y}` | Closed: subject must be exactly `{a: T, b: U}`. |
| `{a: x, b: y, ...}` | Open: subject may carry extra fields (ignored). |
| `{a: x, ...rest}` | Open: `rest` binds the remaining fields as a record. |
| `{}` | Closed empty: only the empty record `{}`. |
| `{...}` | Open empty: any record. |
| `{...rest}` | Captures the whole record as `rest`. |

**Field shorthand.** `{a, b}` is sugar for `{a: a, b: b}` — in a pattern it binds the field value to a variable of the same name. Mix freely:

```
when p is {name, role: r, ...} { ... }
```

**Row polymorphism.** Records are row-polymorphic, so a function destructuring a few fields stays generic over the others. `fun p { p.name }` is typed `{name: a, ...} -> a`.

### Exhaustiveness

A record pattern whose sub-patterns are all bindings covers every value of the subject's type, so `when` doesn't need an `else`:

```
def midpoint = fun pt {
    when pt is {x: xv, y: yv} {              # binding-only sub-patterns
        (xv + yv) / 2
    }
}
```

A sub-pattern that can fail (literal, constructor, list, …) makes the arm refutable, and `when` then requires an `else`:

```
when r is {code: 0, ...} {                   # literal 0 can fail
    "zero"
} else {
    "other"
}
```

At function boundaries, prefer the open form (`{name: n, ...}`) — analogous to `[head, ...]` opting into "more elements allowed."

### Nested destructuring

```
def split-out-name = fun p {
    when p is {name: n, ...rest} {
        (n, rest)        # rest carries every field of `p` except `name`
    }
}

def main = fun {
    let (n, r) = split-out-name {name: "reid", age: 28, role: "engineer"}
    print n           # reid
    print r.role      # engineer (r : {age: int, role: string})
}
```

## List patterns

List patterns destructure `list a` in `when`/`if`/`while`; the always-matches forms also work in `let`.

```
when items is [] {
    "empty"
} is [n, ...rest] {
    "$(to-string n) and $(to-string (size rest)) more"
}
```

| Pattern | Matches |
| - | - |
| `[]` | The empty list. |
| `[a, b, c]` | A list of exactly three elements. |
| `[a, b, ...]` | Any list with at least two elements; tail discarded. |
| `[a, b, ...rest]` | Same, binding the remaining elements as `list a`. |
| `[...rest]` / `[...]` | Any list; only the first binds. |

Elements use the full sub-pattern syntax:

```
when xs is [(x, y), ...] { print "first pair: $(to-string x), $(to-string y)" }
when xs is [some n, ...] { print "first slot has $(to-string n)" }
when xs is [0, _, ...]   { print "starts with zero" }
```

### Exhaustiveness on lists

`when` on a `list a` is exhaustive when both halves are covered — typically `[]` plus a pattern like `[_, ...]`:

```
def length = fun xs {
    when xs is [] {
        0
    } is [_, ...rest] {
        1 + length rest
    }
}
```

An `else` branch also covers everything, as usual.

### List patterns in `let`

Only when they always match — i.e. `[...]` or `[...rest]` with no required elements:

```
let [...everything] = items   # binds `everything` to all of `items`
```

Use `when`/`if` for any pattern that can fail.
