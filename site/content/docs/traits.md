+++
title = "Traits"
description = "Ad-hoc polymorphism. Trait methods dispatch on argument type."
weight = 9
+++

## Declaring a trait

A `trait` declares a set of method signatures over a type parameter. Method signatures use `::` (the type-annotation operator).

```pluma
trait showable a {
    show :: a -> string
}
```

## Implementing a trait

`implement TRAIT TYPE { … }` declares an instance — the implementation for a particular type.

```pluma
implement showable int {
    def show = fun x { to-string x }
}

implement showable bool {
    def show = fun b {
        when b is true { "yes" } else { "no" }
    }
}
```

## Default methods

A trait method that has a fallback body uses `def` inside the trait body (same shape as a real def):

```pluma
trait greeter a {
    name  :: a -> string
    greet :: a -> string

    def greet = fun x { "hello, $(name x)" }
}
```

## Bare-name dispatch

Trait methods are reachable under their **bare names** in the module that declares the trait, and in any module that has the trait in scope:

```pluma
print (show 42)           # int instance
print (show true)         # bool instance
```

Dispatch is by argument type — the compiler picks the instance from the call site's types. Local `def`s shadow trait methods with the same name. When two in-scope traits export the same method name, you get an ambiguity error and must qualify:

```pluma
print (showable.show 42)  # explicit form, always legal
```

## Exporting a trait

A trait is **private to its module** by default, like every other definition. Mark it `public` to let other modules use it, exactly as you would a `def` or an `enum`:

```pluma
# shapes.pa
public trait drawable a {
    draw :: a -> string
}
```

A module that `use`s `shapes` brings the trait into scope — its methods then dispatch by bare name, the same as a local trait (Rust's `use Trait`):

```pluma
# main.pa
use shapes

implement drawable circle { def draw = fun c { "○" } }

def render = fun c { draw c }   # bare dispatch — picks the circle instance
```

When you need to disambiguate (two in-scope traits sharing a method name), name the trait through its module — `module.trait.method` — mirroring how an imported variant is `module.enum.variant`:

```pluma
print (shapes.drawable.draw my-circle)
```

Default methods (below) travel with the trait: an instance in the importing module that omits one inherits the trait's default body. Instances are always visible across modules regardless of visibility — dispatch has to stay globally coherent, and the orphan rule keeps that sound — so there's no `public` on an `implement`.

## Prelude traits

Four traits are visible in every module:

| Trait | Methods | Instances |
| - | - | - |
| `numeric` | `add`, `sub`, `mul`, `div`, `negate` | `int`, `float` |
| `ord` | `compare` | `int`, `float`, `string`; parametric on `option a`, `result a b` |
| `hash` | `hash` | `int`, `float`, `string`, `bool`; parametric on `option a`, `result a b` |
| `wire` | `encode`, `decode` | structural — auto-derived from a type's shape (no concrete instances) |

So `compare 1 2`, `hash "key"`, `add 1.5 2.5` all just work.

## Constrained instances

Instances can carry constraints with `where`:

```pluma
implement ord (option a) where (ord a) {
    def compare = fun x y {
        when x is some xv {
            when y is some yv { compare xv yv }  # bare — dispatches on `a`
            is none { gt }
        }
        is none {
            when y is some _v { lt }
            is none { eq }
        }
    }
}
```
