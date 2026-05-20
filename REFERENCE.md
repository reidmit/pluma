# language reference

## basic values

```
let some-int = 10
let some-float = 1.23
let some-string = "hello"
let some-bool = true
```

## bytes

`bytes` is an immutable sequence of 8-bit values — no UTF-8 invariant. Use it for binary data, wire formats, hashes, and anything else that isn't necessarily text. Literal syntax uses single quotes:

```
let greeting = 'hello'
let png-header = '\x89PNG\r\n\x1a\n'
let empty-bytes = ''
```

`bytes` and `string` are distinct types with no implicit conversion. The bridge is explicit:

- `string.to-bytes :: string -> bytes` — UTF-8 encode (infallible).
- `bytes.to-string :: bytes -> result string string` — UTF-8 decode (fallible).

Bytes literals support `\\`, `\'`, `\0`, `\t`, `\r`, `\n`, and `\xNN` (two hex digits). They do **not** support `$(...)` interpolation — interpolation lives on `string`. Non-ASCII characters that appear inside a bytes literal are encoded as their UTF-8 bytes.

`'...'` patterns work in `when` / `if` / `while`:

```
when method is 'GET' { ... }
is 'POST' { ... }
```

`core.bytes` exposes a parallel surface to `core.string`:

```
use core.bytes

bytes.length b              # int — byte count
bytes.is-empty b            # bool
bytes.at b i                # option int — none if out of bounds
bytes.concat a b
bytes.slice b start end     # clamp-to-bounds; end < start gives ''
bytes.contains haystack needle
bytes.starts-with b prefix
bytes.ends-with b suffix
bytes.repeat b n
bytes.reverse b
bytes.to-list b             # list int — one entry per byte
bytes.from-list xs          # result bytes string — errs if any int is < 0 or > 255
bytes.join parts sep        # parts :: list bytes
bytes.split b sep           # empty sep splits into single-byte chunks
```

Hashing and ordering: `bytes` has prelude `hash` and `ord` instances, so `compare 'abc' 'abd'` and using bytes as map keys both work without any setup. Ordering is byte-lexicographic.

Byte-aware I/O lives in `core.io`:

```
io.read-file-bytes path        # result bytes string — survives non-UTF-8 contents
io.write-file-bytes path bytes # result nothing string
io.append-file-bytes path bytes
io.read-all-bytes ()           # drain stdin as bytes
io.write-bytes b               # raw write to stdout, no newline, no Display formatting
io.write-err-bytes b           # same, to stderr
```

The text-side equivalents (`io.read-file`, `io.write-file`, `io.read-all`) still exist and still require UTF-8; reach for the byte-side versions when you're dealing with binary data or when source encoding is uncertain.

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

## refs

a `ref` is a mutable cell. it's the language's only mutation primitive — everything else is immutable. the `ref` module is auto-imported in every module; you don't write `use core.ref`.

```
let counter = ref.new 0
ref.update counter fun n { n + 1 }    # most common form
ref.set counter 100                   # explicit write
print (ref.get counter)               # explicit read
```

`ref.new x` returns `ref a` (where `a` is the type of `x`). `ref.get`, `ref.set`, and `ref.update` operate on the cell:

- `ref.new :: a -> ref a`
- `ref.get :: ref a -> a`
- `ref.set :: ref a -> a -> nothing`
- `ref.update :: ref a -> (a -> a) -> nothing`

`ref.set` and `ref.update` both return `nothing` — mutation is a statement, not an expression. if you want the new value, call `ref.get` after.

equality on refs is **reference identity**: two refs are equal iff they point to the same underlying cell. two distinct cells holding the same value are not equal.

```
let a = ref.new 5
let b = a            # same cell
let c = ref.new 5    # distinct cell

print (a == b)       # true
print (a == c)       # false
```

passing a ref to a function lets that function observe and mutate the cell. this is the intended escape hatch: functions that mutate their arguments must take refs, so the type signature makes the effect visible.

```
def bump = fun r {
	ref.update r fun n { n + 1 }
}

def main = fun {
	let counter = ref.new 0
	bump counter
	bump counter
	print (ref.get counter)    # 2
}
```

`ref` works in any type position — alias bodies, record fields, function signatures.

```
alias counter ref int

alias session {
	id    :: string
	hits  :: ref int
}
```

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

## let destructuring

a `let` binding accepts any irrefutable pattern on the left — identifier, wildcard, tuple, record, and nestings of those. the same shapes used in `if` / `when` / `while`, restricted to patterns that always match.

```
let (a, b) = (1, 2)
let (lo, _, hi) = (0, 50, 100)

let p = {name: "reid", age: 28}
let {name: n, age: a} = p

# nested
let ((x, y), z) = ((10, 20), 30)
let {label: lbl, coords: (cx, cy)} = {label: "origin", coords: (0, 0)}
```

refutable patterns (constructor, literal, string-interpolation) aren't allowed — those can fail to match, which would leave bindings undefined. use `if` or `when` for those cases:

```
# rejected: `some` can fail (the value might be `none`)
# let some x = maybe-value

# instead:
if maybe-value is some x {
  print (to-string x)
}
```

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
  show :: a -> string
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
  name  :: a -> string
  greet :: a -> string

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

## record patterns

record patterns destructure records in `when` / `if` / `while` / `let`. by default they require an **exact** match on the field set; add `, ...` to allow extras (and skip them).

```
let {name: n, age: a} = {name: "reid", age: 28}   # exact: types must match
let {name: n, ...} = {name: "alice", age: 30}     # open: extras ignored

when person is {name: n, role: r} { ... }         # exact `{name, role}` only
when person is {name: n, ...} { ... }             # any record with a `name`
```

- `{a: x, b: y}` — closed: subject must be exactly `{a: T, b: U}`.
- `{a: x, b: y, ...}` — open: subject may carry extra fields (ignored).
- `{a: x, ...rest}` — open: `rest` binds to a record containing whichever
  fields the subject has beyond `a`.
- `{}` — closed empty: matches only the empty record `{}`.
- `{...}` — open empty: matches any record.
- `{...rest}` — captures the whole record as `rest`.

field shorthand: `{a, b}` is sugar for `{a: a, b: b}` — in a literal the value comes from the in-scope `a`/`b`; in a pattern it binds the field value to a variable of the same name. Mix and match freely (`{role, team, level: 1}`, `when p is {name, role: r, ...}`).

```
def split-out-name = fun p {
  when p is {name: n, ...rest} {
    (n, rest)        # rest carries every field of `p` except `name`
  }
}

def main = fun {
  let (n, r) = split-out-name {name: "reid", age: 28, role: "engineer"}
  print n           # reid
  print r.role      # engineer (r : {age: int, role: string})
}
```

records are **row-polymorphic**: a function destructuring a few fields stays
generic over the others. `fun p { p.name }` is typed `{name: a, ...} -> a`,
so it accepts any record with a `name` field.

a record pattern whose sub-patterns are all bindings (`_` or an identifier) covers every value of the subject's type, so `when` doesn't need an `else`:

```
def midpoint = fun pt {
  when pt is {x: xv, y: yv} {              # binding-only sub-patterns
    (xv + yv) / 2
  }
}
```

a sub-pattern that can fail (literal, constructor, list, …) makes the arm refutable, and `when` then requires an `else` or catch-all:

```
when r is {code: 0, ...} {                 # literal 0 can fail
  "zero"
} else {
  "other"
}
```

if you want flexibility at function boundaries, prefer the open form (`{name: n, ...}`) — that's analogous to how `[head, ...]` opts into "more elements allowed."

## list patterns

list patterns destructure `list a` in `when` / `if` / `while` (and the always-matches forms work in `let`).

```
when items is [] {
  "empty"
} is [n, ...rest] {
  "$(to-string n) and $(to-string (size rest)) more"
}
```

- `[]` — matches the empty list.
- `[a, b, c]` — matches a list of exactly three elements. Element patterns can be anything (literals, identifiers, wildcards, nested patterns).
- `[a, b, ...]` — matches any list with at least two elements; doesn't capture the tail.
- `[a, b, ...rest]` — same, but binds the remaining elements as `list a`.
- `[...rest]` and `[...]` — match any list; only the second binds.

elements use the same sub-pattern syntax as other patterns:

```
when xs is [(x, y), ...] { print "first pair: $(to-string x), $(to-string y)" }
when xs is [some n, ...] { print "first slot has $(to-string n)" }
when xs is [0, _, ...]   { print "starts with zero" }
```

`when` on a `list a` is exhaustive when both halves are covered — typically `[]` plus a pattern like `[_, ...]` that catches every non-empty case:

```
def length = fun xs {
  when xs is [] {
    0
  } is [_, ...rest] {
    1 + length rest
  }
}
```

an `else` branch also covers everything, as usual.

list patterns in `let` work only when they always match — i.e. `[...]` or `[...rest]` with no required elements:

```
let [...everything] = items   # binds `everything` to all of `items`
```

use `when` / `if` for any pattern that can fail.