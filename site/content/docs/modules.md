+++
title = "Modules"
description = "Each `.pa` file is a module. `use` brings another module in as a namespace."
weight = 10
+++

## Imports

`use` at the top of a module brings another module in as a namespace. Dotted paths resolve relative to the project root.

```
use math
use sub.utils
use other.utils as utils2   # avoids collision with `sub.utils` above

def four = math.add 2 2
def value = utils.something
def alt = utils2.something
```

## Crossing module boundaries

Values, enums, and aliases all cross module boundaries.

```
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

```
let c :: colors.color = colors.color.red
```

## Cycles

Imports are cycle-checked. A diagnostic is emitted if module `A`'s import graph eventually reaches itself.
