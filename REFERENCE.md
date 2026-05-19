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

only allowed at top level

can be values or types

```
def name "reid"

def greet fun name {
  print "hello, $(name)!"
}
```

## alias types

```
def person alias {
  name: string
  age: int
}

def number-list alias list int
```

## enum types

enums are nominal: two enums with the same shape are distinct, and references within an enum's body (e.g. `tree` inside `tree`'s `node` variant) are allowed.

```
def color enum {
  red
  green
  blue
}

def tree enum {
  empty
  node int tree tree
}

def bool enum {
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

enums can take type parameters, listed space-separated after `enum`. variants reference them by name.

```
def option enum a {
  some a
  none
}

def result enum a b {
  ok a
  err b
}

def pair enum a b {
  both a b
  left a
  right b
}
```

instantiate with space-separated type args in any type position (alias bodies, record fields, etc.):

```
def maybe-int alias option int

def named-list alias {
  name: string
  items: list (option int)
}
```

multi-arg type contexts (variant params) are non-greedy — wrap generic applications in parens there: `def container enum a { holds (option a) }`.

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

## module imports

`use` at the top of a module brings another module in as a namespace. dotted paths resolve relative to the project root.

```
use math
use sub.utils
use other.utils as utils2   # avoids collision with `sub.utils` above

def four math.add 2 2
def value utils.something
def alt utils2.something
```

values, enums, and aliases all cross module boundaries.

```
use shapes
use colors

def themed alias {
  primary: colors.color
  shape: shapes.circle
}

def my-favorite colors.color.red
```

- in type positions: `module.type-name` refers to an imported enum or alias.
- in value positions: `module.enum-name.variant` accesses a variant; `module.alias-name` is the alias constructor.

imports are cycle-checked.

## if expressions

single-armed pattern matching

not limited to booleans!

for multiple cases, use when

always evaluates to `nothing`

```
if some-value is 47 {
  print "ok cool"
}

if some-animal is dog name {
  print "it's a dog called $(name)"
}

if result is ok value {
  print "success! got $(value)"
}
```

## when expressions

must be exhaustive! all cases must be covered

can use `is _` as a catch-all, "else" case

evaluates to value of first matching case

all cases must have the same type

```
when some-value is 47 {
  print "ok cool"
} is _ {
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