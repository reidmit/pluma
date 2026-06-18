# Control flow

Pluma's everyday control flow will feel familiar if you've used any imperative
language: `if` chooses between branches, and `while` repeats. This page covers
those two. The next page shows how `if` is really a small slice of a more
powerful matching construct.

## if

An `if` runs a block when a condition holds, and an optional `else` block
otherwise.

```pluma
if score > 100 {
	print "high score!"
} else {
	print "keep going"
}
```

The condition is just a boolean expression (`score > 100`, `ready`, `count == 0`),
exactly what you'd write in any other language.

An `if` is also an expression: it produces a value you can name. Both branches
have to produce the same type:

```pluma
let label = if ready { "go" } else { "wait" }
```

And it chains with `else if`, just as you'd expect:

```pluma
let grade = if score >= 90 {
	"A"
} else if score >= 80 {
	"B"
} else {
	"C"
}
```

## while

A `while` repeats its block for as long as the condition holds. Since names don't
change on their own, a loop that counts needs a `ref`, a mutable box, covered in
[Mutable state](/docs/tour/state):

```pluma
let i = ref.new 0
while ref.get i < 3 {
	print (to-string (ref.get i))
	ref.update i (fun n { n + 1 })
}
```

Much of the time you won't reach for `while` at all: iterating over a list or
transforming a collection is clearer with the list functions from
[Lists, tuples & records](/docs/tour/collections). Save `while` for genuine
loops where the number of steps isn't known up front.

Next: [Pattern matching](/docs/tour/pattern-matching), where `if` turns out to be
more powerful than it first appears.
