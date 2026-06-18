# Control flow and matching

Pluma has three forms for choosing what to do — `if`, `when`, and `while` — and
all of them share one idea: a *pattern*, a shape a value might match. This page
covers the three forms and then patterns themselves.

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

An `if` is also an expression — it produces a value you can name:

```pluma
let label = if ready { "go" } else { "wait" }
```

And it chains with `else if`, just as you'd expect.

## when

When there are several cases, `when` is clearer. It checks the cases top to
bottom and takes the first that matches. A `when` must cover every possibility,
so you can't quietly forget a case — the compiler points it out.

```pluma
when http-status is 200 {
	"ok"
} is 404 {
	"not found"
} else {
	"something else"
}
```

Each `is ...` clause names a pattern to test the subject against. Only the first
arm carries the subject (`http-status`); the rest just list their patterns. The
`else` is the catch-all for anything unmatched — and because `when` is checked
for completeness, you need one unless the patterns already cover every case.

## while

A `while` repeats its block for as long as the condition holds. Since names don't
change on their own, a loop that counts needs a `ref` — a mutable box, covered in
[Mutable state](/docs/tour/state):

```pluma
let i = ref.new 0
while ref.get i < 3 {
	print (to-string (ref.get i))
	ref.update i (fun n { n + 1 })
}
```

## Patterns and destructuring

Each `is ...` clause above is a pattern. Patterns do more than test a value —
they pull it apart and name the pieces. You can destructure a tuple or record
right in a `let`:

```pluma
let (x, y) = (10, 20)
let {name: n, age: a} = person
```

Some patterns *always* match — naming a value, or splitting a tuple of known
size. Those are the ones a `let` accepts, because a `let` has nowhere to go if
the match fails. Other patterns *might* fail, like checking for a specific
number; those belong in an `if`, `when`, or `while`, where there's a branch to
take when they don't.

Lists can be matched by shape, too — the empty list, or a first element and "the
rest":

```pluma
when items is [] {
	"nothing here"
} is [first, ...rest] {
	"starts with $(to-string first)"
}
```

## if is

The `is pattern` part of an `if` is optional. Leave it out and the condition is
just a boolean test — `if score > 100` is exactly what it looks like. Put it back
and `if` becomes a one-branch match, handy when you only care about a single
case:

```pluma
if lookup users id is some user {
	greet user
} else {
	print "no such user"
}
```

Here the `else` is the no-match case. One syntax wrinkle to know: a `{` in the
subject always opens the body, so if you want to test a record literal, wrap it
in parentheses — `if f ({ ... }) { ... }`. The same applies to `while`.

Next: [Enums](/docs/tour/enums), where patterns really earn their keep.
