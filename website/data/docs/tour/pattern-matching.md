# Pattern matching

The [previous page](/docs/tour/control-flow) introduced `if` as a boolean test.
That's the everyday face of it, but underneath, `if` is a *pattern match*, and
the boolean form is just the common case. Once you see the full shape, the same
construct pulls values apart, reaches inside options and results, and handles any
number of branches.

## if is

The full form of an `if` is `if subject is pattern`. A *pattern* describes a
shape a value might have; the branch runs when the subject matches it.

```pluma
if score is 100 {
	print "perfect!"
} else {
	print "not quite"
}
```

So why does `if score > 100` work without an `is`? Because the `is pattern` is
optional, and when you leave it out it defaults to `is true`. A plain boolean
`if` is shorthand for matching against the pattern `true`:

```pluma
if ready { ... }            // these two are
if ready is true { ... }    // exactly the same
```

That's the whole trick: the familiar boolean `if` is the one-pattern case of a
more general tool.

## Matching and binding

Patterns are at their best when they pull a value apart *and* name the pieces in
one step. An `option` either holds a value or doesn't; matching on `some` reaches
inside and binds the contents to a name you can use in the branch:

```pluma
if lookup users id is some user {
	greet user
} else {
	print "no such user"
}
```

Here `user` exists only inside the first branch: that's the value that was
wrapped in `some`. The `else` is the no-match case (here, `none`). A `result`
works the same way with `ok` and `err`:

```pluma
if parse-int text is ok n {
	print "got $(to-string n)"
} else {
	print "not a number"
}
```

One syntax wrinkle to know: a `{` in the subject always opens the body, so if you
want to test a record literal, wrap it in parentheses: `if f ({ ... }) { ... }`.
The same applies to `while`.

## when

`if` gives you two branches: match or don't. When there are several cases,
`when` is clearer. It checks each `is` clause top to bottom and takes the first
that matches:

```pluma
when http-status is 200 {
	"ok"
} is 404 {
	"not found"
} else {
	"something else"
}
```

Only the first arm carries the subject (`http-status`); the rest just list their
patterns. A `when` must cover *every* possibility: if the patterns don't already
account for each case, you need an `else`, and the compiler will tell you when one
is missing. That completeness check is what makes `when` safe to lean on: you
can't quietly forget a case.

## Destructuring

Some patterns *always* match: naming a value, or splitting apart a tuple or
record of known shape. Because they can't fail, you can use them right in a
`let`:

```pluma
let (x, y) = (10, 20)
let {name: n, age: a} = person
```

This is *destructuring*: the pattern mirrors the structure of the value and binds
its parts. It's the same pattern syntax as in `if` and `when`, just used where
the match is guaranteed.

Other patterns *might* fail, like checking for a specific number or a particular
variant. Those belong in an `if`, `when`, or `while`, where there's a branch to
take when they don't match. A `let` has nowhere to go on failure, so it only
accepts the always-match kind.

Lists can be matched by shape, too: the empty list, or a first element and "the
rest":

```pluma
when items is [] {
	"nothing here"
} is [first, ...rest] {
	"starts with $(to-string first)"
}
```

Next: [Enums](/docs/tour/enums), where patterns really come into their own.
