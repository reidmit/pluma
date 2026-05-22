+++
title = "Traits"
description = "Ad-hoc polymorphism. Trait methods dispatch on argument type."
weight = 9
+++

## Declaring a trait

A `trait` declares a set of method signatures over a type parameter. Method signatures use `::` (the type-annotation operator).

```
trait showable a {
    show :: a -> string
}
```

## Implementing a trait

`implement TRAIT TYPE { … }` declares an instance — the implementation for a particular type.

```
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

```
trait greeter a {
    name  :: a -> string
    greet :: a -> string

    def greet = fun x { "hello, $(name x)" }
}
```

## Bare-name dispatch

Trait methods are reachable under their **bare names** in the module that declares the trait, and in any module that has the trait in scope:

```
print (show 42)           # int instance
print (show true)         # bool instance
```

Dispatch is by argument type — the compiler picks the instance from the call site's types. Local `def`s (and bare enum variants) shadow trait methods with the same name. When two in-scope traits export the same method name, you get an ambiguity error and must qualify:

```
print (showable.show 42)  # explicit form, always legal
```

## Prelude traits

Three traits are visible in every module:

| Trait | Methods | Instances |
| - | - | - |
| `numeric` | `add`, `sub`, `mul`, `div`, `negate` | `int`, `float` |
| `ord` | `compare` | `int`, `float`, `string`; parametric on `option a`, `result a b` |
| `hash` | `hash` | `int`, `float`, `string`, `bool`; parametric on `option a`, `result a b` |

So `compare 1 2`, `hash "key"`, `add 1.5 2.5` all just work.

## Constrained instances

Instances can carry constraints with `where`:

```
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
