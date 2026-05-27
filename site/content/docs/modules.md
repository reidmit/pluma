+++
title = "Modules"
description = "Each `.pa` file is a module. `use` brings another module in as a namespace."
weight = 10
+++

## Imports

`use` at the top of a module brings another module in as a namespace. Dotted paths resolve relative to the project root.

```pluma
use math
use sub.utils
use other.utils as utils2   # avoids collision with `sub.utils` above

def four = math.add 2 2
def value = utils.something
def alt = utils2.something
```

## Visibility

Top-level definitions are **private by default** — visible only inside their own module. To let another module `use` something, mark it `public`:

```pluma
public def add = fun x y { x + y }   # exported
def helper = fun n { n + n }         # private — internal to this module
```

Reaching for a private name from another module is an error (`helper is private to module …`). Privacy is checked only when type-checking; it never changes what a program does at runtime.

### Opaque types

`opaque` applies to an enum and is the middle rung of the ladder: it exports the type's *name* but hides its *constructors*. Other modules can name the type and pass values around, but can't build or pattern-match one directly — so the module stays in control of every value's invariants. Expose `public` functions as the way in and out:

```pluma
opaque enum uuid {
    bytes (list int)
}

public def parse :: fun string -> option uuid = fun s { … }   # the only way to make one
public def to-string :: fun uuid -> string = fun u { … }      # …and to read one
```

A caller can write `def id :: ids.uuid = ids.parse raw ?? …` but `ids.uuid.bytes [...]` is rejected. There are three rungs in all — bare (private), `opaque` (type exported, constructors hidden), and `public` (everything exported). `opaque` is enum-only; `public`/`opaque` on a `def`, `alias`, `trait`, or `implement` other than `public def`/`public alias` is a parse error.

## Crossing module boundaries

Public values, enums, and aliases all cross module boundaries.

```pluma
use shapes
use colors

alias themed {
    primary :: colors.color
    shape   :: shapes.circle
}

def my-favorite = red
```

| Form | Meaning |
| - | - |
| `module.type-name` (in type positions) | An imported enum or alias. |
| `module.value-name` (in value positions) | An imported top-level value or function. |
| `module.enum-name.variant` | Access a variant of an imported enum. |
| `module.alias-name` | The alias constructor for an imported record alias. |

## Bare variants

Variants of imported enums can be used bare when the subject type is known or when there's no ambiguity. When two enums in scope (one local, one imported, or two imported) share a variant name, the local-module enum wins; if both are non-local, you'll get an `AmbiguousVariant` error and need to qualify:

```pluma
let c :: colors.color = colors.color.red
```

## Cycles

Imports are cycle-checked. A diagnostic is emitted if module `A`'s import graph eventually reaches itself.
