+++
title = "Basic values"
description = "The primitive and built-in compound values you'll work with day to day."
weight = 1
+++

## Primitive literals

```pluma
let some-int    = 10
let some-float  = 1.23
let some-string = "hello"
let some-bool   = true
```

Integers and floats are distinct types with distinct operators (see [Operators](@/docs/operators.md)). Strings are UTF-8 text; for binary data, see [Bytes](@/docs/bytes.md).

## String interpolation

```pluma
let name = "reid"
let message = "hello $(name)"
```

Any expression can appear inside `$(…)`. The result is converted to text via the `showable` trait.

## Regex literals

```pluma
let some-regex = ` "a" ("b" | "c") "d" `
```

See [Regexes](@/docs/regexes.md) for the full syntax.

## Tuples

Heterogeneous, fixed-size containers.

```pluma
let some-tuple = (1, "reid", true)
```

## Lists

Homogeneous, variable-size containers.

```pluma
let some-list = [1, 3, 0, 10]

let list-across-lines = [
    "one",
    "two",
    "three",
]
```

## Dicts

Mutable, insertion-ordered hash dicts (key/value tables). `insert` and `remove` change the dict in place (and hand it back, for chaining), so two bindings to one dict are two views of the same table — like a `ref`. There's no dict literal syntax — construct one through `std/dict`:

```pluma
use std/dict

let m = dict.empty ()
let _ = dict.insert m "alice" 30
let _ = dict.insert m "bob" 25

when (dict.lookup m "alice") is some n { print n } is none { print 0 }
```

The key type must have a `hash` instance. `int`, `float`, `string`, `bool`, `option a`, and `result a b` are all wired up out of the box; user enums and records get a hash instance the moment they declare one with `for hash on …`. Operations that need to bucket a key (`insert`, `lookup`, `remove`, `contains-key`, `from-entries`, `merge`) carry a `where (hash k)` constraint and resolve the dictionary automatically at the call site.

Iteration (`keys`, `values`, `entries`, `fold`, `map`, `filter`) is in insertion order. `insert`/`remove` mutate in place; the transformers `map`, `filter`, and `merge` instead build a new dict and leave their inputs alone. `from-entries` and `merge` are right-wins on duplicate keys. `==` on dicts is structural and order-independent. `size` returns the entry count.

See `std/dict` for the full surface: `empty`, `insert`, `lookup`, `remove`, `contains-key`, `size`, `keys`, `values`, `entries`, `from-entries`, `merge`, `map`, `filter`, `fold`.

## Refs

A `ref` is a mutable cell — the simplest way to hold state that outlives a single expression. Most values are immutable; the deliberate exceptions are `ref`, a `dict` (which `insert`/`remove` mutate in place), and the in-place `list.set`/`list.push`. The `ref` module is auto-imported in every module; you don't write `use std/ref`.

```pluma
let counter = ref.new 0
ref.update counter fun n { n + 1 }    # most common form
ref.set counter 100                   # explicit write
print (ref.get counter)               # explicit read
```

Signatures:

- `ref.new :: a -> ref a`
- `ref.get :: ref a -> a`
- `ref.set :: ref a -> a -> nothing`
- `ref.update :: ref a -> (a -> a) -> nothing`

`ref.set` and `ref.update` both return `nothing` — mutation is a statement, not an expression. If you want the new value, call `ref.get` after.

Equality on refs is **reference identity**: two refs are equal iff they point to the same underlying cell.

```pluma
let a = ref.new 5
let b = a            # same cell
let c = ref.new 5    # distinct cell

print (a == b)       # true
print (a == c)       # false
```

Passing a ref to a function lets that function observe and mutate the cell. This is the intended escape hatch: functions that mutate their arguments must take refs, so the type signature makes the effect visible.

```pluma
def bump = fun r {
    ref.update r fun n { n + 1 }
}

def main = fun {
    let counter = ref.new 0
    bump counter
    bump counter
    print (ref.get counter)    # 2
}
```

`ref` works in any type position — alias bodies, record fields, function signatures.

```pluma
alias counter ref int

alias session {
    id    :: string
    hits  :: ref int
}
```

## Records

Keyed by identifiers, no dynamic keys.

```pluma
let some-record = {name: "reid", age: 28}

let record-across-lines = {
    name: "reid"
    age: 28
}

print some-record.name
```

Field shorthand: `{a, b}` is sugar for `{a: a, b: b}` — in a literal the value comes from the in-scope `a`/`b`.

Records are **row-polymorphic**: a function destructuring a few fields stays generic over the others. `fun p { p.name }` is typed `{name: a, ...} -> a`, so it accepts any record with a `name` field.

## Functions

```pluma
let add-one = fun x {
    x + 1
}

let print-each = fun list {
    each list fun item {
        print (to-string item)
    }
}
```

{% note() %}
**Pluma is uncurried.** `add 5` is an arity error, not partial application. To partially apply, wrap explicitly:

```pluma
let add-five = fun y { add 5 y }
```
{% end %}
