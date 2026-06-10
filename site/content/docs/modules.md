+++
title = "Modules"
description = "Each `.pa` file is a module. `use` brings another module in as a namespace."
weight = 10
+++

## Imports

`use` at the top of a module brings another module in as a namespace. Slash-separated paths resolve relative to the project root.

```pluma
use math
use sub/utils
use other/utils as utils2   # avoids collision with `sub/utils` above

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

A caller can write `def id :: ids.uuid = ids.parse raw ?? …` but `ids.uuid.bytes [...]` is rejected. There are three rungs in all — bare (private), `opaque` (type exported, constructors hidden), and `public` (everything exported). `public` applies to a `def`, `alias`, `enum`, or `trait`; `opaque` is enum-only. Either keyword on an `implement` — or `opaque` on anything but an enum — is a parse error (an instance's visibility is never written: instances are always exported, see [Traits](@/docs/traits.md)).

## Crossing module boundaries

Public values, enums, and aliases all cross module boundaries.

```pluma
use shapes
use colors

alias themed {
    primary :: colors.color
    shape   :: shapes.circle
}

def my-favorite = colors.color.red
```

| Form | Meaning |
| - | - |
| `module.type-name` (in type positions) | An imported enum or alias. |
| `module.value-name` (in value positions) | An imported top-level value or function. |
| `module.enum-name.variant` | Access a variant of an imported enum. |
| `module.alias-name` | The alias constructor for an imported record alias. |
| `module.trait-name.method` | Call a method of an imported trait (the explicit form of bare dispatch). |

## Variants are always qualified

Everything an import brings in is reached *through the name you imported* — there are no bare names injected into your scope. That holds for enum variants too: a variant is always written `enum.variant`, mirroring how its type is named.

- A **local** enum is reached through its bare name: `color.red`.
- An **imported** enum is reached through the module, like its type: `use colors` makes the type `colors.color`, so the variant is `colors.color.red`.

This is true in expressions *and* patterns (`when c is colors.color.red { … }`). A bare variant name is rejected with a diagnostic that names the exact qualified form to write. (Prelude variants — `some`, `none`, `ok`, `err` — are the sole exception and stay bare.)

```pluma
let c :: colors.color = colors.color.red

when c is colors.color.red {
    "primary"
} else {
    "other"
}
```

## Ambient namespaces: `using`

Qualification keeps provenance explicit, but in a block that leans hard on one module — a CSS ruleset, a view tree — the repeated prefix is noise. A `using <namespace> { … }` block makes that namespace *ambient*: inside it, a leading-dot `.member` resolves in the named module, so `css.color` becomes `.color`. It's the scoped, opt-in counterpart to `use` (which binds a namespace for the whole file).

```pluma
use std/css

def card :: css.ruleset = using css {
    .rule [.padding (.rem 1.0), .background (.hex "#0b1020")]
}
```

Only the leading dot is ambient: `.color` is `css.color`, but `.color.foo` is `(css.color).foo`, and a bare name (no dot) is an ordinary lexical lookup — so other namespaces stay qualified inside the block (`signal.get`, `list.map`). The block's value is its last expression, like a `fun` body; `let` bindings inside it are scoped to the block.

Blocks nest, and the innermost ambient wins:

```pluma
using view {
    .div [] [.text (using string { .join parts ", " })]
}
```

A leading `.member` outside any `using` block is an error (E0031) — write it qualified, or wrap it in a `using`. The dot makes provenance legible *and* greppable: unlike a wildcard import, you can always see which names are coming from the ambient namespace. (A line that *starts* with `.` is its own statement, never a field-access chain continuing the line above.)

## A module and its principal type

It's common for a module to be named after the one type it's built around — `shapes/circle` exporting a `circle`, the way `std/task` is the home of `task`. To avoid `circle.circle` stutter, the **eponymous type** — an enum named like the module's last path segment — is brought into scope *bare* when you `use` the module:

```pluma
use shapes/circle

def grow :: fun circle -> circle = …   # bare `circle` in a type position
let c = circle.radius 1.0              # `circle.variant` still constructs
def a = circle.area c                  # `circle.fn` is still the module function
```

The one name plays both roles, disambiguated by syntax: `circle` in a type position is the type; `circle.x` is a variant or a module member. Under an alias the type rides the alias (`use shapes/circle as disk` makes `disk` the bare type). A local declaration or a prelude type of the same name wins, so this never shadows a built-in. (The prelude's `option`/`result` are the same overlap — a bare type plus an auto-imported module of helpers.)

## Cycles

Imports are cycle-checked. A diagnostic is emitted if module `A`'s import graph eventually reaches itself.
