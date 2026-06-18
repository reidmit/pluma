# Traits

Sometimes you want one name to work across many types. `to-string` turns an int
into text — it'd be nice if it worked for your own types too. `+` adds two ints,
and also two floats. A `trait` is how Pluma expresses that: it describes a
*capability* — a set of operations — that different types can provide.

```pluma
trait to-text a {
	to-text :: fun a -> string
}
```

The `a` is a stand-in for "any type that has this capability." The trait says: a
type counts as `to-text` if it supplies a `to-text` function from itself to a
string.

## Giving a type a capability

You grant a type a capability with `implement`, filling in the operations for
that specific type:

```pluma
implement to-text bool {
	def to-text = fun b {
		if b { "yes" } else { "no" }
	}
}
```

Now `to-text true` finds the right implementation automatically, chosen from the
type of its argument:

```pluma
to-text true    # => "yes"
to-text false   # => "no"
```

Add an `implement to-text` for another type and the same `to-text` name covers it
too. The compiler picks the matching one by the argument's type — you never say
which.

## Where you already rely on traits

This isn't an exotic feature you opt into; Pluma's built-in operators are traits.
`+`, `-`, `*`, and `/` come from a `numeric` trait that both `int` and `float`
implement, which is why the same `+` works on either — and why a generic `fun x {
x + x }` works for any numeric type. The comparison operators come from an `ord`
trait, so `<` works on anything that knows how to compare itself.

That's also why `2 + 3.5` is a type error: `+` needs both sides to be the *same*
numeric type, and there's no implementation that mixes them.

## Requiring a capability

A function can demand that its type argument carry a capability, using `where`:

```pluma
def announce :: fun a -> string where (to-text a) = fun x {
	"the answer is $(to-text x)"
}
```

The `where (to-text a)` reads "for any type `a` that has the `to-text`
capability." Inside, you're free to call `to-text x`, and the compiler guarantees
every caller passes a type that supports it. This is the same machinery
`std/error` uses: a precise error erases into the general `error` type only if it
has a `describe` capability — opt-in, checked, no surprises.

Next: [Modules](/docs/tour/modules).
