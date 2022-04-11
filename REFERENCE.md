# Pluma language reference

This document should be considered the source of truth for Pluma syntax and semantics. The compiler and tests may not always be up-to-date.

## Program structure

Files must be UTF-8.

### Conventions

Use kebab-case, all-lowercase identifiers (e.g. `my-name` instead of `myName` or `my_name`).

Use kebab-case for separating words in file and directory names.

Use tabs for indentation, not spaces.

### Comments

Pluma only has single-line comments beginning with `#`.

```pluma
# This is a comment on its own line

let x = 47 # This is a comment on the same line as a statement
```

### Packages & modules

A module is a single file. Modules are identified by their path (e.g. `path/to/module`), relative to the project root.

Standard library modules always start with `std/`.

Third-party dependency modules will always start with `pkg/`.

User module paths should not begin with `std/` or `pkg/`, because the compiler will ignore them.

#### Imports

A Pluma module (file) can import other modules.

```pluma
use std/fs
use pkg/some-third-party-module
use path/to/local/module
```

Importing a module without a qualifier adds all exported identifiers from that module to the current scope.

Qualifiers can be used to add a namespace prefix to imported identifiers.

```pluma
use @fs std/fs

let thing = @fs read-file "hi.txt"
```

#### Exports

## Built-in value types

Pluma has a handful of built-in value types.

Note that there is no built-in boolean value type.

### Integers

Integers in Pluma have built-in type `int`.

#### Decimal integers

```pluma
47
```

#### Hex integers

```pluma
0xfacade
0X123
0xbeefface
```

#### Octal integers

```pluma
0o755
0O755
```

#### Binary integers

```pluma
0b101
0B101
```

### Strings

Strings have built-in type `string`. Strings are always double-quoted.

```pluma
"hello, world"
```

#### String interpolations

Strings may contain interpolations. Interpolations may contain any single expression that has type `string`.

```pluma
"hello, $(name)"
```

### Characters

Characters have built-in type `char`. Characters are wrapped in single-quotes.

```pluma
'a'
```

### Regular expressions

Regular expressions the built-in type `regex`.

Pluma has a special syntax for defining regular expressions.

```pluma
/ "hello" /
/ "a" "b"+ "c" /
```

## Tuple types

### Empty tuple

```pluma
()
```

### Unlabeled tuple

```pluma
(true, false)
(1, 2, "hello")
(
  1,
  2,
  3,
)
```

### Labeled tuple

```pluma
(name: "Reid", age: 28)
(
  a: 1,
  b: 2,
  c: 3,
)
```

## Lists

Lists have type `list<e>` where `e` is the type of each element.

If a list is mutable, it can grow, shrink, and have its elements updated.

```pluma
[]
[1, 2, 3]
["a", "b", "c"]
```

## Dictionaries

Dictionaries have type `dict<k, v>` where `k` is the type of each key, and `v` is the type of each element.

If a dictionary is mutable, elements can be added/removed/updated.

```pluma
[:]
["key1": 1, "key2": 2]
[1: "ok", 100: "also ok", 47: "hey"]
```

## Function types

Functions are wrapped in curly braces (`{` and `}`). Functions _always_ take a single parameter and return a single value. They have the type `x -> y`, where `x` is the parameter type and `y` is the return type.

Tuple types can be used to pass multiple values as the single parameter of a function. For example, `{ (a, b) -> a + b }` might have type `(int, int) -> int`.

Empty tuples (`()`) can be used if a function does not take any meaningful parameters or does not return any meaningful values; the empty function `{}` has type `() -> ()`.

Within a function body, if the parameter is used, it should be bound first-thing before a `->` arrow. An irrefutable pattern can be used to destructure the parameter into its component parts, if desired.

```pluma
{}
{ print "hello world" }
{ a -> a + 1 }
{ (a, b) -> a + b }
```

## User-defined types

### Structs

```pluma
struct person (
  name :: string
  age :: int
)

struct person (name :: string, age :: int)

struct int-wrapper (int)

struct box<a> where a :: any (a)
```

### Enums

```pluma
enum color {
  red
  green
  blue
  custom (r :: int, g :: int, b :: int)
}

enum boolean { true, false }

enum node<a> where a :: any {
  leaf
  node (a)
}
```

### Aliases

```pluma
alias list-of-ints = list<int>

alias color = @some-module some-other-name-for-color
```

### Traits

```pluma
trait any {}

trait person-like {
  .name :: string
  .age :: int
}

trait growable {
  _ grow _ :: (mut self, int) -> nil
}
```

## Statements

### Let bindings

Let bindings add new names to the current scope.

By default, let bindings are immutable.

```pluma
let name = "Reid"
let age = 28
```

Let bindings support multi-part identifier names. These must be bound to functions, and they must have `_` underscores marking where they can take parameters when called.

```pluma
let if _ then _ = {
  (predicate, then-block) -> match predicate {
    true -> do then-block
    false -> ()
  }
}

let _ plus _ :: (int, int) -> int = {
  (x, y) -> x + y
}
```

#### Type annotations

Let bindings can have type annotations, marked with `::`. Sometimes these are unnecessary, if the compiler can infer the type from the value, but sometimes they are required.

```pluma
let name :: string = "Reid"

let if _ then _ :: (bool, () -> ()) -> () = {
  (predicate, then-block) -> match predicate {
    true -> do then-block
    false -> ()
  }
}
```

#### Mutable let bindings

Mutable bindings have the keyword `mut` after `let`. Values defined in this way have built-in type `mut<a>`, where `a` is the type of the value.

```pluma
let mut name = "Reid"
name :: mut<string>

let mut p = person ("reid", 28)
p :: mut<person>
```

### Get bindings

Get bindings destructure existing values using pattern matching and bind their components to new names. These new names are added to the current scope.

Note that the patterns here must be _irrefutable_. It is a compilation error to use a refutable binding in a `get` statement.

```pluma
get (first, second) = some-tuple
get (first, _) = some-tuple
get (_, second) = some-tuple

get person (name, age) = p
get person (_, age) = p
```

#### Mutable get bindings

The following binds a new mutable binding, `first`, with the current value of `some-tuple.0`.

```pluma
get (mut first, _) = some-tuple

first = "updated" # allowed, since this new binding is mutable, but doesn't change original value in first element of some-tuple
```

## Expressions

### Type assertions

```pluma
some-value :: int
idk :: list<string>
```

### Reassignments

Values that are mutable (have type `mut<something>`) can be reassigned.

Mutable tuples, structs, lists, and dictionaries can have their fields/entries updated.

```pluma
a = 10
some-tuple.field = "updated"
person.name = "reid"
```

### Match expressions

Match expressions compare a value against one or more refutable patterns.

Cases are checked in order; the branch of the first pattern that matches will be executed.

Cases must have a single expression after the `->` arrow. All cases must evaluate to values of the same type.

Cases must be exhaustive. For enum types, this means there must be a pattern that matches each variant.

A catch-all pattern (`_`) can be used as a default case to match anything.

```pluma
match color {
  red -> print "it's red"
  green -> print "it's green"
  blue -> print "it's blue"
  rgb(r, g, b) -> print ("it's %s, %s, %s" | format [r, g, b])
  _ -> print "it's something else?"
}

do check-if-active | match {
  true -> print "active"
  false -> print "inactive"
}
```

### Call expressions

```pluma
let greet _ = { name -> print "hello, $(name)" }
let speak _ = { print "welcome" }
let do _ = { f -> f () }

greet "reid"
speak ()
do speak
```

```pluma
let if _ then _ else _ = {
  # ...
}

if is-active then "yep" else "nope"

if is-active then {
  print "wow"
} else {
  print "whelp"
}
```

```pluma
let _ plus _ = { ... }

1 plus 1

let _ uppercase = { ... }
let _ replace _ with _ = { ... }

("reid" uppercase) replace "I" with "E"
```

### Pipe expressions

## Pattern matching

This section describes Pluma's pattern matching semantics in more detail. Pattern matching has already been seen in `get` bindings and `match` expressions.

### Irrefutable patterns

An irrefutable pattern will always match the value of the expression in question.

```pluma
let tuple = (1, 2, 3)
get (first, second, third) = tuple
```

### Refutable patterns

A refutable pattern is one that may or may not match the value of the expression in question.

```pluma
enum maybe<a> { some(a), none }

# ERROR: refutable pattern used in get! we don't know that this is a "some"
get some (val) = some-maybe-value

# OKAY: we cover all possible cases here with refutable patterns
some-maybe-value | match {
  some (val) -> print "yep, it's something"
  none -> print "nope"
}

do get-random-int | match {
  0 -> print "zero"
  1 -> print "one"
  _ -> print "something else"
}
```

## Examples

```pluma
# multi-arg function (all args merged into single tuple)
def add _ to _ :: (int, int) -> int {
  (x, y) => x + y
}
# called like:
add 46 to 1
```

```pluma
# "zero-arg" function (really single empty arg)
def random-color _ :: nil -> color {
  # ...
}
# called like:
random-color ()
```

```pluma
# function with receiver
def _ | say-name :: person -> nil {
  self => print ("my name is " ++ self.name)
}
# called like:
let reid = person("reid", 27)
reid | say-name
```

```pluma
# function chaining
let transformed = "reid" | to-uppercase | split-chars | filter (is-not-ascii _)
```

```pluma
# passing around functions as first-class values
let list2 = [1, 2, 3] | map (add1 _)
let list2 = [1, 2, 3] | map { el => add1 el }
[(1, 2), (3, 4)] | map (add _ to _)
people | map (_ | say-name)
let add-tuple = add _ to _
add-tuple (1, 2)
```
