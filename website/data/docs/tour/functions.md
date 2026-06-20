# Functions

A function is written `fun`, then its argument names, then a block. The block's
last expression is what the function gives back. There's no `return` keyword.

```pluma
let double = fun n { n * 2 }
let add = fun x y { x + y }
let greet = fun { print "hi" }   # no arguments
```

You call a function by writing it next to its arguments, separated by spaces, with
no parentheses around the whole call. Parentheses are only for grouping
sub-expressions.

```pluma
double 21          # => 42
add 2 3            # => 5
print (double 21)  # parens group the inner call, then print the result
```

This paren-free style is the single biggest adjustment coming from most
languages. `add 2 3`, not `add(2, 3)`. The parentheses in `print (double 21)`
aren't a call; they group `double 21` so it's evaluated first and its result
handed to `print`.

## Every argument at once

Here's the rule that catches newcomers: a function takes *all* of its arguments
at once. `add 2` isn't "add with one argument filled in"; it's an error,
because `add` wants two. If you want a version of `add` with the first argument
fixed, write a small function that says so:

```pluma
let add-five = fun y { add 5 y }
add-five 10   # => 15
```

That's a one-line habit, and it keeps calls unambiguous: when you see `f x y`,
you know `f` is being called with two arguments, every time.

## Zero arguments: () versus {}

A function with no arguments is written `fun { ... }`. To *call* it, you pass it
an empty argument list, written `()`:

```pluma
let say-hi = fun { print "hi" }
say-hi ()   # call it: prints "hi"
```

Watch the two empty brackets. `{}` is a block, the body of a function or branch.
`()` is the empty argument list you use to call a zero-argument function. So
`io.read ()` and `dict.empty ()` *call*; a bare `{ ... }` is code to run, not a
call.

## Naming a function with def

A top-level function is just a `def` whose value is a `fun`:

```pluma
def square = fun n { n * n }
```

When you want to write the type down, the annotation goes on the name with `::`,
and the function type reads `fun ARGS -> RESULT`:

```pluma
def square :: fun int -> int = fun n { n * n }
def add :: fun int int -> int = fun x y { x + y }
```

Compound argument types get parentheses so the arrow is unambiguous:
`fun (list int) -> int` is a function from a list of ints to an int.

## Functions are values

A function is a value like any other, so you can pass one to another function.
Many standard tools take a function as an argument: `list.map` applies one to
every element:

```pluma
use std/list

list.map [1, 2, 3] (fun n { n * 10 })   # => [10, 20, 30]
```

The function argument is often written inline like that, right at the call. When
it's the last argument you can drop the surrounding parens entirely, which is why
you'll see `list.map [1, 2, 3] fun n { n * 10 }` throughout the docs: same call,
less punctuation.

Next: [Strings and text](/docs/tour/strings).
