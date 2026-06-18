# The basics

This is the start of a tour that builds Pluma up from the ground, each page
leaning on the ones before it. You don't need any functional-programming
background, just the ability to program in *some* language. We'll define the
rest as we go.

A Pluma program is a list of definitions. One of them is named `main`, and
running the program runs it.

```pluma
def main = fun {
	print "hello, world"
}
```

`def` gives a name to a value. `fun { ... }` is a function, here one that takes
no arguments and prints a greeting. `print` writes a line to the screen. Run it
and you get `hello, world`. Everything that follows is about the values you can
name, the types they carry, and the handful of forms for putting them together.

## Values and types

Pluma has the building blocks you'd expect: whole numbers, decimals, text, and
true-or-false.

```pluma
let count = 10        # int
let price = 1.99      # float
let name  = "Ada"     # string
let ready = true      # bool
```

Every value has a type, and Pluma checks those types before your program runs,
so a whole class of mistakes is caught early. But you rarely write the types out
yourself: Pluma works them out from how each value is used. The `# int` comments
above are just for us; the compiler already knew.

Whole numbers (`int`) and decimals (`float`) are kept apart on purpose. There's
no silent conversion between them, so `2 + 3.5` is a type error rather than a
surprise. If you want a float, say so: `2.0 + 3.5`.

## Naming things: let and def

There are two ways to give something a name. `def` works at the top level of a
file; `let` works inside a function.

```pluma
def pi = 3.14159          # top level

def main = fun {
	let radius = 2.0      # local to this function
	let area = pi * radius * radius
	print (to-string area)
}
```

Both use `=` to join the name and the value. A `def` is visible to the whole
file, and, if you mark it `public`, to other files (more on that in
[Modules](/docs/tour/modules)). A `let` is visible only from where it appears to
the end of its function.

These names aren't variables in the change-it-later sense: once bound, a name
keeps its value for good. That immutability is a feature: you can read a
function top to bottom and trust that a name means one thing throughout. When you
genuinely need something that changes over time, you reach for a `ref`, covered
in [Mutable state](/docs/tour/state).

## Writing types down

You rarely need to, but you can annotate any binding with `::` and a type, and
the compiler will hold you to it:

```pluma
let count :: int = 10
def pi :: float = 3.14159
```

Annotations are most useful on top-level `def`s, where they double as
documentation and pin down exactly what a function accepts and returns. The next
page, [Functions](/docs/tour/functions), shows that form.

Up next: how functions are written and called, including the one calling
convention that trips up newcomers.
