# Enums

An `enum` defines a type by listing the shapes its values can take. Each shape is
a *variant*. The simplest enums are just a set of named choices:

```pluma
enum color {
	red
	green
	blue
}
```

Variants can also carry data. A shape might be a circle with a radius, or a
rectangle with a width and a height:

```pluma
enum shape {
	circle float
	rect float float
}

let s = shape.circle 2.0
```

You reach a variant through its enum's name — `color.red`, `shape.circle` — the
same way you reach a field through a record. (The one exception is `option` and
`result`, whose variants are written bare; see [Errors and missing
values](/docs/reference/errors).)

## Using an enum: match on it

To use an enum value, match it with `when`. Because `when` is checked for
completeness, you're guaranteed to handle every variant — add a case to the enum
later and the compiler flags every match that forgot it:

```pluma
def area = fun s {
	when s is shape.circle r {
		3.14159 * r * r
	} is shape.rect w h {
		w * h
	}
}

area (shape.circle 2.0)   # => 12.56636
area (shape.rect 3.0 4.0) # => 12.0
```

The pattern `shape.circle r` does two things at once: it checks that `s` is a
circle, and it names the radius `r` so the branch can use it. That's the payoff
from the [patterns](/docs/tour/control-flow) page — a variant carrying data is
unpacked the moment you match it.

## Type parameters

Enums can take type parameters, so one definition works for any element type.
Here's a binary tree of anything:

```pluma
enum tree a {
	leaf
	node (tree a) a (tree a)
}
```

The `a` is a stand-in for whatever type the tree holds — `tree int`, `tree
string`, even `tree (tree bool)`. This is how the built-in `option` and `result`
are defined, and how you'd write your own generic container. Compound variant
fields get parentheses, the same as in a type annotation: `node (tree a) a (tree
a)` is a node holding a left subtree, a value, and a right subtree.

Next: [Mutable state with ref](/docs/tour/state).
