+++
title = "Bindings"
description = "Names bound to values, locally with `let` and at module scope with `def`."
weight = 6
+++

## Top-level `def`

`def` binds a name to a value at the top level. `=` separates the name from the expression — same as `let` does locally.

```
def name = "reid"

def greet = fun name {
    print "hello, $(name)!"
}

def main = fun {
    greet name
}
```

The right-hand side is any expression — string, int, record, function literal, function call. `def` is value-only; type definitions use their own keywords ([`alias`](@/docs/types.md), [`enum`](@/docs/types.md), [`trait`](@/docs/traits.md)).

## Local `let`

```
let x = 10
let greeting = "hello, world"
let doubled = fun n { n * 2 }
```

### Destructuring

A `let` binding accepts any **irrefutable** pattern on the left — identifier, wildcard, tuple, record, and nestings of those. The same shapes used in `if`/`when`/`while`, restricted to patterns that always match.

```
let (a, b) = (1, 2)
let (lo, _, hi) = (0, 50, 100)

let p = {name: "reid", age: 28}
let {name: n, age: a} = p

# nested
let ((x, y), z) = ((10, 20), 30)
let {label: lbl, coords: (cx, cy)} = {label: "origin", coords: (0, 0)}
```

Refutable patterns (constructor, literal, string-interpolation) aren't allowed — those can fail to match, which would leave bindings undefined. Use `if` or `when` for those cases:

```
# rejected: `some` can fail (the value might be `none`)
# let some x = maybe-value

# instead:
if maybe-value is some x {
    print (to-string x)
}
```

See [Patterns](@/docs/patterns.md) for the full pattern grammar.

## Type annotations with `::`

`::` annotates a name with its type. Used inside `alias` bodies (record-style types), `trait` method signatures, and explicit annotations on `def`.

`::` is distinct from `:` so the two roles never collide:

| Operator | Role | Example |
| - | - | - |
| `:` | Field name → value (record literals, patterns) | `{name: "reid"}` |
| `::` | Name has type X (annotations) | `name :: string` |

### Annotating a `def`

The general shape is `def name :: TYPE = expr`. For function types, the form is `fun T1 T2 -> R`, with parens around compound argument types.

```
def pi :: float = 3.14159

def add :: fun int int -> int =
    fun x y { x + y }

def map-fst :: fun (list (a, b)) -> list a =
    fun pairs {
        map pairs fun (x, _) { x }
    }
```
