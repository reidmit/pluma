# Type aliases and nominal types

The tour built up records and enums. Two rules round out the type system: how to
give a type a name, and when two types count as the same.

## Aliases

An `alias` introduces a named type. There are two forms: a record shape (fields
use `::`), and a bare type expression:

```pluma
alias point {
	x :: float
	y :: float
}

alias number-list list int
```

An alias is just a name for a type; it introduces no new identity. `point` and
`{x: float, y: float}` are the very same type, interchangeable everywhere.

## Nominal enums, structural records

Enums are *nominal*: each carries a fully-qualified name internally, so two enums
with identical variants but different names (or modules) are distinct types and
won't unify. That's why a variant is always reached through its enum (`color.red`,
`shape.circle`), mirroring how you name the type itself.

Records, by contrast, are *structural*: there's no named record type. A function
that reads a couple of fields stays generic over whatever else the record carries.

```pluma
# name-of has type {name: a, ...} -> a
def name-of = fun r { r.name }
```

The two built-in enums `option` and `result` are seeded into every module (no
`use` needed) and they are the one exception to qualified variants: `some`,
`none`, `ok`, and `err` are written bare.
