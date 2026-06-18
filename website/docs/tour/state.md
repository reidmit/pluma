# Mutable state with ref

Names in Pluma don't change once bound, which is what lets you read a function
top to bottom and trust that each name means one thing throughout. But sometimes
you really do need a value that updates over time — a counter, a running total,
an accumulator inside a loop. That's a `ref`: a small box you can read from and
write to.

```pluma
let count = ref.new 0            # a box holding 0
ref.update count (fun n { n + 1 })
ref.set count 100
ref.get count                    # => 100
```

Four operations cover it: `ref.new` makes a box around a starting value,
`ref.get` reads what's inside, `ref.set` replaces it, and `ref.update` applies a
function to the current contents — the usual choice when the new value depends on
the old one, like incrementing.

`ref` is auto-imported, so you can write `ref.new` without a `use`. It's the one
deliberate exception to immutability in the language. And because reaching into a
box shows up in a function's type, you can always tell from a signature which
functions can change state and which can't — the escape hatch is visible, never
hidden.

## A worked example

A `while` loop is the classic place a `ref` earns its keep, since the loop needs
something that changes each time around:

```pluma
let total = ref.new 0
let i = ref.new 1
while ref.get i <= 5 {
	ref.update total (fun t { t + ref.get i })
	ref.update i (fun n { n + 1 })
}
ref.get total   # => 15
```

That said, reach for a `ref` only when you mean it. Much of the time a transform
over a list (`list.map`, `list.fold`) says the same thing without a mutable box,
and reads more directly. A `ref` is for genuine state that outlives a single
expression.

Next: [Traits](/docs/tour/traits).
