# Pluma language reference

## Conventions

- Files must be UTF-8
- Kebab-case, all-lowercase identifiers preferred
- Use tabs for indentaion

## Examples

```pluma
# value assignments
let a = 1
let is-cool = true
let is-uncool = false
let list = [1, 2, 3]
let dict = ["a": 1, "b": 2]
let unlabeled-tuple = ("hey", 2)
let labeled-tuple = (a: 1, b: "hey")
let char = 'a'
```

```pluma
# value assignments with type annotations (optional)
let a :: int = 1
```

```pluma
# mutable values
let mut a = 1
a = a + 1
```

```pluma
# single-arg function (int)
def add1 _ :: int -> int = {
  x => x + 1
}
# called like:
add1 47
```

```pluma
# single-arg function (tuple)
def add _ :: (int, int) -> int = {
  (x, y) => x + y
}
# called like:
add (46, 1)
```

```pluma
# multi-arg function (all args merged into single tuple)
def add _ to _ :: (int, int) -> int = {
  (x, y) => x + y
}
# called like:
add 46 to 1
```

```pluma
# "zero-arg" function (really single empty arg)
def random-color :: nil -> color = {
  # ...
}
# called like:
random-color ()
```

```pluma
# function with receiver
def _ | say-name :: person -> nil = {
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

```pluma
# destructuring assignment
let (a, b) = (1, 2)
let (a, _) = (1, 2)
let person(name, age) = p
# dicts + lists can NOT be destructured, since they don't have fixed elements
#   e.g. let [a, b] = someList # can't work, because someList may have only 1 element
```

```pluma
# match expressions
get-color | match {
  case color.red => print "it's red"
  case color.green => print "it's green"
  case color.blue => print "it's blue"
  case color.rgb(r, g, b) => print ("it's %s, %s, %s" | format [r, g, b])
  case _ => print "it's something else?"
}
```

```pluma
# built-in types
nil
bool
float
int
string
char
_ -> _
any
```

```pluma
# struct types
type person = struct (
  name: string
  age: int
)

let p = person ("reid", 27)
let p = person (name: "reid", age: 27)
```

```pluma
# enum types
type bool = enum {
  true
  false
}
let t = true

type color = enum {
  red :: color
  green :: color
  blue :: color
  r _ g _ b _ :: (int, int, int) -> color
  hex _ :: string -> color
}
let r = red
let c = custom (100, 200, 255)

type maybe a' = enum {
  some _ :: a' -> maybe a'
  none
}

let r :: maybe string = none
let o = some "reid"
```

```pluma
# traits
type any = trait {}

type person-like = trait {
  .name :: string
  .age :: int
}

type growable = trait {
  | grow _ :: (mut self, int) -> nil
}
```

```pluma
# alias types
type bool-list = alias list bool
let bs :: bool-list = [true, false, true]

type identity-func a' where a' :: any = alias a' -> a'
```

```pluma
# person.pa

# this is private (syntax tbd)
type person = person (name: string, age: int, counter: int)

# this is public/exported
def new-person _ :: (string, int) -> person = {
  init => person (
    name: init.name,
    age: init.age,
    counter: 0
  )
}

def _ | grow :: mut person -> nil = {
  self => self.age = self.age + 1
}

# another file...

let me = new-person ("reid", 27)

# INVALID, since `person` type name isn't exported:
let me2 = person (name: "reid", age: 27, counter: 10)
```

```pluma
# colors.pa

type color = enum {
  red :: color
  green :: color
  blue :: color
  r _ g _ b _ :: (int, int, int) -> color
  hex _ :: string -> color
}

def new-color _ :: () -> color = {
  red
}

def random-color _ :: () -> color = {
  random-int-between 0 and 4 | match {
    case 0 => red
    case 1 => green
    case 2 => blue
    case _ => hex "#000"
  }
}

# another file...

let rc = random-color ()

rc | match {
  case red => print "it's red"
  case _ => print "it's not red"
}

if rc == red then {
  print "it's red"
} else {
  print "it's not red"
}
```

### `let` vs `def`

At first glance, `let` and `def` keywords look similar, but there are important differences.

- `let` allows destructuring with patterns
- `def` allows parameter placeholders (`_`s) and multi-part names

In practice, you should usually use `def` for definitions that use the block syntax (`def thing _ = { ... }`).

```pluma
# preferred:
def add _ = { (x, y) => x + y }

# possible, but less flexible:
let add = { (x, y) => x + y}
```
