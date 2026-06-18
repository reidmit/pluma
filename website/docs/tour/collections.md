# Lists, tuples, and records

Three ways to hold more than one value at a time, each for a different shape of
data.

## Lists

A list holds any number of values, all of the same type. Write one with square
brackets.

```pluma
let primes = [2, 3, 5, 7, 11]
let names = ["Ada", "Alan", "Grace"]
```

You can build a new list from an existing one with the spread `...`, which copies
every element of another list into this one:

```pluma
let more = [1, ...primes]   # => [1, 2, 3, 5, 7, 11]
```

The `std/list` module has the usual tools — `list.map` to transform every
element, `list.filter` to keep some, `list.fold` to boil a list down to a single
value, and plenty more.

```pluma
use std/list

list.map [1, 2, 3] (fun n { n * 10 })       # => [10, 20, 30]
list.filter [1, 2, 3, 4] (fun n { n > 2 })  # => [3, 4]
```

## Tuples

A tuple holds a fixed number of values that may have *different* types. Write one
with parentheses and commas.

```pluma
let point = (1, 2)
let row = ("Ada", 36, true)
```

Reach for a tuple when you want to return two or three related values from a
function without inventing a name for each — a pair of coordinates, a key and a
value. When the pieces deserve names, use a record instead.

## Records

A record holds named fields. Write one with braces, and read a field with a dot.

```pluma
let person = {name: "Ada", age: 36}
person.name        # => "Ada"
```

Build a changed copy with the spread `...`, naming only the fields you want
different — the rest are carried over from the original:

```pluma
let older = {...person, age: 37}   # => {name: "Ada", age: 37}
```

Records are *structural*, which has a pleasant consequence: a function that reads
a couple of fields works on **any** record that has them, whatever else it
carries. This one accepts anything with a `name`:

```pluma
def label = fun thing { thing.name }

label {name: "Ada", age: 36}          # => "Ada"
label {name: "Mars", radius: 3389.5}  # => "Mars"
```

There's no separate "record type" to declare before you can use one — the shape
*is* the type. The [Type aliases](/docs/reference/aliases) page covers how to
give a record shape a name when you want one, and why records and enums differ on
what counts as "the same type."

These three are the built-in shapes. When you need to look values up by a key or
test membership, the standard library adds [dictionaries and
sets](/docs/stdlib/dict-set).

Next: [Control flow and matching](/docs/tour/control-flow).
