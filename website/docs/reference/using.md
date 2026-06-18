# Using blocks

Some modules are built to be called over and over in one stretch of code — a
styling module, a module of HTML builders. Writing the module name in front of
every call gets noisy:

```pluma
use std/view

view.div [view.styled card] [
	view.h2 [] [view.text "Title"],
	view.p [] [view.text "Body"],
]
```

A `using` block removes that repetition. Inside `using M { ... }`, a name written
with a leading dot — `.div`, `.text` — resolves in module `M`, so the prefix
drops away:

```pluma
use std/view

using view {
	.div [.styled card] [
		.h2 [] [.text "Title"],
		.p [] [.text "Body"],
	]
}
```

Both snippets mean exactly the same thing. The leading `.div` inside the block is
shorthand for `view.div`; `.text` is `view.text`. Nothing else changes — it's
purely a way to set an ambient module so the calls read cleanly.

## It's an expression

A `using` block is an expression: it evaluates its body and produces that value,
so you can name the result or return it directly.

```pluma
use std/list

let xs = using list {
	.map [-2, 5, -8] (fun n { n * 2 })   # .map is list.map here
}
# xs => [-4, 10, -16]
```

## Switching the ambient, and reaching outside it

Blocks nest, and the innermost one wins — handy when one DSL calls into another.
Here the outer block makes `.map` resolve in `std/list`, while a nested block
switches the ambient to `std/math` so `.abs` resolves there:

```pluma
use std/list
use std/math

using list {
	.map [-2, 5, -8] (fun n {
		using math { .abs n }   # .abs is math.abs
	})
}
# => [2, 5, 8]
```

Only the leading dot is affected. A fully-qualified name still works inside a
block — you can write `math.abs n` in the middle of a `using list` block and it
resolves the normal way — so the ambient never traps you. A leading dot is the
*only* thing that looks at the surrounding `using`.

## The leading dot needs a block

A leading-dot name is meaningful only inside a `using` block. Written on its own,
with no enclosing block to resolve against, it's a compile error — there's no
ambient module to look in:

```pluma
def main = fun {
	let x = .foo   # error: leading-dot member outside a using block
	x
}
```

This is different from the *postfix* dot you already know. `person.name` reads a
record field, and `list.map` names a module member — both attach the dot to
something on the left. A `using` block's leading dot has nothing on its left;
the block itself stands in for what would be there.

## Where you'll meet it

You'll see `using` most around the styling and view modules, where it's the
natural way to write markup and CSS without a prefix on every node:

```pluma
def card :: css.ruleset = using css {
	.rule [.padding (.rem 1.0), .border-radius (.px 8.0)]
}
```

It's an ordinary language feature, though, not tied to those modules — any module
you call repeatedly in one place is a candidate. Reach for it when a prefix is
repeating itself; skip it when a single qualified call is clearer on its own.
