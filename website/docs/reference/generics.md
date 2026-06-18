# Generics

A *generic* function or type is one that works for many types at once, instead of
being tied to a single one. Pluma leans on this heavily — `list.map` works on a
list of anything, `option` holds a value of any type — and most of the time it
happens without you writing a thing, because type inference fills in the details.
This page makes the mechanism explicit.

## Type variables

The key piece is a **type variable**: a lowercase name, usually a single letter
like `a` or `b`, that stands in for "some type — any type, but the same one each
place it appears." You've already seen them in the signatures throughout these
docs:

```pluma
def first :: fun (list a) -> option a = fun xs {
	when xs is [head, ...] { some head } else { none }
}
```

Read `a` as a placeholder. The signature says: for *whatever* element type the
list holds, `first` takes a `list` of that type and returns an `option` of the
same type. Because the same `a` appears in both spots, the two are linked — a
list of ints gives back `option int`, a list of strings gives back `option
string`:

```pluma
first [1, 2, 3]     # => some 1     (here a is int)
first ["a", "b"]    # => some "a"   (here a is string)
```

One definition, every element type. There's no separate `first-int` and
`first-string` — the single generic `first` covers them all, and the compiler
checks each use against the right type.

## You rarely write them

That signature is optional. Leave it off and inference works out the most general
type on its own:

```pluma
def name-of = fun r { r.name }
```

The compiler figures out that `name-of` takes any record with a `name` field and
returns whatever that field holds — a type it would write `fun {name: a, ...} ->
a`. You get the generality for free; annotations are for when you want to pin a
type down as documentation or to catch a mistake earlier. (See [Type
aliases](/docs/reference/aliases) for why records are generic over their *extra*
fields like this.)

## Generic types

Types take type variables too. An `enum` lists them after its name, and they
stand in for the contents:

```pluma
enum pair a b {
	both a b
}
```

`pair` isn't one type but a whole family — `pair int string`, `pair bool bool`,
and so on. A function over it carries the variables through:

```pluma
def swap :: fun (pair a b) -> pair b a = fun p {
	when p is pair.both x y { pair.both y x }
}

swap (pair.both 1 "x")   # => pair.both "x" 1   (a pair int string becomes pair string int)
```

The two built-in enums you use constantly, `option` and `result`, are defined
exactly this way — `option a`, `result a e`. When you write a generic type as an
annotation, you supply its arguments in order: `list int`, `option string`, `dict
string int`. Compound arguments get parentheses so they read unambiguously —
`option (list int)` is an option holding a list of ints.

## Requiring a capability

A plain type variable means "literally any type," so inside such a function you
can only move the value around — you can't add it, compare it, or print it,
because not every type supports those. When you need an operation, you constrain
the variable to types that provide it, with `where` and a
[trait](/docs/tour/traits):

```pluma
def largest :: fun (list a) -> option a where (ord a) = fun xs {
	list.max xs
}
```

`where (ord a)` reads "for any type `a` that knows how to compare itself." That's
the bridge between "works for anything" and "needs a specific ability" — the
variable stays generic, but only over the types that can do what the body asks.
The [Traits](/docs/tour/traits) page covers defining and implementing those
abilities.

## See also

- **[Type aliases and nominal types](/docs/reference/aliases)** — how named and
  structural types interact with type variables.
- **[Traits](/docs/tour/traits)** — the capabilities a `where` clause requires.
