# language reference

## basic values

```
let some-int = 10
let some-float = 1.23
let some-string = "hello"
let some-bool = true
```

## regex literals

```
let some-regex = / "a" ("b" | "c") "d" /
```

## tuples

heterogeneous, fixed-size containers

```
let some-tuple = (1, "reid", true)
```

## lists

homogeneous, variable-size containers

```
let some-list = [1, 3, 0, 10]
let list-across-lines = [
  "one"
  "two"
  "three"
]
```

## maps

immutable, insertion-ordered hash maps. there's no map literal syntax — construct one through `core.map`:

```
use core.map

let m = map.empty ()
let m = map.insert m "alice" 30
let m = map.insert m "bob" 25

when (map.lookup m "alice") is some n { print n } is none { print 0 }
```

the key type must have a `hash` instance — `int`, `float`, `string`, `bool`, `option a`, and `result a b` are all wired up out of the box; user enums and records get a hash instance the moment they declare one with `for hash on ...`. operations that need to bucket a key (`insert`, `lookup`, `remove`, `contains-key`, `from-entries`, `merge`) carry a `where (hash k)` constraint and resolve the dictionary automatically at the call site.

iteration (`keys`, `values`, `entries`, `fold`, `map`, `filter`) is in insertion order. `from-entries` and `merge` are right-wins on duplicate keys. `==` on maps is structural and order-independent.

see `core.map` for the full surface: `empty`, `insert`, `lookup`, `remove`, `contains-key`, `size`, `keys`, `values`, `entries`, `from-entries`, `merge`, `map`, `filter`, `fold`.

## records

keyed by identifiers, no dynamic keys

```
let some-record = {name: "reid", age: 28}
let record-across-lines = {
  name: "reid"
  age: 28
}
print some-record.name
```

## functions

```
let add-one = fun x {
  x + 1
}

let print-each = fun list {
  each list fun item {
    print (to-string item)
  }
}
```

## string interpolations

```
let name = "reid"
let message = "hello $(name)"
```

## definitions

`def` binds a name to a value at the top level. `=` separates the name from the expression — same as `let` does locally.

```
def name = "reid"

def greet = fun name {
  print "hello, $(name)!"
}

def main = fun {
  greet name
}
```

The right-hand side is any expression — string, int, record, function literal, function call. `def` is value-only; type definitions use their own keywords (`alias`, `enum`, `trait`).

## type annotations

`::` annotates a name with its type. Used inside `alias` bodies (record-style types) and `trait` method signatures. Distinct from `:` so the two roles never collide:

| operator | role | example |
| - | - | - |
| `:`  | field name → value (record literals, patterns) | `{name: "reid"}` |
| `::` | name has type X (annotations) | `name :: string` |

## alias types

```
alias person {
  name :: string
  age  :: int
}

alias number-list list int
```

The first form is a record-type alias (fields use `::`). The second is a bare type expression alias.

## enum types

enums are nominal: two enums with the same shape are distinct, and references within an enum's body (e.g. `tree` inside `tree`'s `node` variant) are allowed.

```
enum color {
  red
  green
  blue
}

enum tree {
  empty
  node int tree tree
}

enum bool {
  true
  false
}
```

variants are accessed by qualifying with the enum name. zero-arg variants are values of the enum type; payload variants are constructor functions.

```
let c = color.red                          # c : color
let t = tree.node 1 tree.empty tree.empty
```

bare variant names also work when unambiguous (`red` instead of `color.red`). if two enums in scope share a variant name, the local-module enum wins; if both are non-local, you get an `AmbiguousVariant` error and need to qualify.

### generic enums

enums can take type parameters, listed space-separated after the name. variants reference them by name.

```
enum option a {
  some a
  none
}

enum result a b {
  ok a
  err b
}

enum pair a b {
  both a b
  left a
  right b
}
```

instantiate with space-separated type args in any type position (alias bodies, record fields, etc.):

```
alias maybe-int option int

alias named-list {
  name  :: string
  items :: list (option int)
}
```

multi-arg type contexts (variant params) are non-greedy — wrap generic applications in parens there: `enum container a { holds (option a) }`.

### prelude enums

`option` and `result` are seeded into every module. no `use` needed; their variants (`some`, `none`, `ok`, `err`) work bare:

```
let n = some 5             # n : option int
let nothing = none         # nothing : option a
let outcome = ok 42        # outcome : result int b
let oops = err "boom"      # oops : result a string

when outcome is ok v {
  print v
} is err msg {
  print msg
}
```

## traits

a `trait` declares a set of method signatures over a type parameter. method signatures use `::` (the type-annotation operator). `implement TRAIT TYPE { ... }` declares an instance — the implementation for a particular type.

```
trait showable a {
  show :: fun a -> string
}

implement showable int {
  def show = fun x { to-string x }
}

implement showable bool {
  def show = fun b {
    when b is true { "yes" } else { "no" }
  }
}
```

a trait method that has a fallback body uses `def` inside the trait body (same shape as a real def):

```
trait greeter a {
  name  :: fun a -> string
  greet :: fun a -> string

  def greet = fun x { "hello, $(name x)" }
}
```

trait methods are reachable under their **bare names** in the module that declares the trait, and in any module that has the trait in scope:

```
print (show 42)           # int instance
print (show true)         # bool instance
```

dispatch is by argument type — the compiler picks the instance from the call site's types. local `def`s (and bare enum variants) shadow trait methods with the same name. when two in-scope traits export the same method name, you get an ambiguity error and must qualify:

```
print (showable.show 42)  # explicit form, always legal
```

the prelude ships three traits visible in every module:

- `numeric` — `add`, `sub`, `mul`, `div`, `negate` (instances on `int`, `float`)
- `ord` — `compare` (instances on `int`, `float`, `string`; parametric on `option a`, `result a b`)
- `hash` — `hash` (instances on `int`, `float`, `string`, `bool`; parametric on `option a`, `result a b`)

so `compare 1 2`, `hash "key"`, `add 1.5 2.5` all just work.

instances can carry constraints with `where`:

```
implement ord (option a) where (ord a) {
  def compare = fun x y {
    when x is some xv {
      when y is some yv { compare xv yv }  # bare — dispatches on `a`
      is none { gt }
    }
    is none {
      when y is some _v { lt }
      is none { eq }
    }
  }
}
```

## module imports

`use` at the top of a module brings another module in as a namespace. dotted paths resolve relative to the project root.

```
use math
use sub.utils
use other.utils as utils2   # avoids collision with `sub.utils` above

def four = math.add 2 2
def value = utils.something
def alt = utils2.something
```

values, enums, and aliases all cross module boundaries.

```
use shapes
use colors

alias themed {
  primary :: colors.color
  shape   :: shapes.circle
}

def my-favorite = red
```

- in type positions: `module.type-name` refers to an imported enum or alias.
- in value positions: `module.enum-name.variant` accesses a variant; `module.alias-name` is the alias constructor.

imports are cycle-checked.

## if expressions

single-armed pattern matching with an optional `else` arm

not limited to booleans!

for multiple cases, use when

without `else` it evaluates to `nothing`; with `else` it evaluates to the
common type of both branches

```
if some-value is 47 {
  print "ok cool"
}

if some-animal is dog name {
  print "it's a dog called $(name)"
}

# `else` runs when the pattern doesn't match
if result is ok value {
  print "success! got $(value)"
} else {
  print "something went wrong"
}

# used as a value
let label = if n is some v { "got $(to-string v)" } else { "none" }
```

## when expressions

must be exhaustive! all cases must be covered

`else` is the catch-all branch (equivalent to `is _`); use whichever reads better

evaluates to value of first matching case

all cases must have the same type

```
when some-value is 47 {
  print "ok cool"
} else {
  print "it's something else"
}

when result is ok value {
  print "success! got $(value)"
} is error message {
  print "failed: $(message)"
}
```

## while expressions

uses pattern matching!

```
while some-value is true {
  print "ya"
}

let iterator = iterate names
while (get-next iterator) is some name {
  print "name: $(name)"
}
```