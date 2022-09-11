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

## records

```
let some-record = {name: "reid", age: 28}
let record-across-lines = {
  name: "reid"
  age: 28
}
```

## functions

```
let add-one = fun (x) {
  x + 1
}

let print-each = fun (list) {
  each list fun (item) {
    print (to-string item)
  }
}
```

## string interpolations

```
let name = "reid"
let message = "hello ${name}"
```

## definitions

only allowed at top level

can be values or types

```
def name = "reid"

def greet = fun (name) {
  print "hello, ${name}!"
}
```

## alias types

```
def person = alias {
  name: string
  age: int
}

def number-list = alias list int
```

## enum types

```
def color = enum {
  red
  green
  blue
}

def tree = enum {
  empty
  node int tree tree
}

def bool = enum {
  true
  false
}
```

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
  print "it's a dog called ${name}"
}

if result is ok value {
  print "success! got ${value}"
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
  print "success! got ${value}"
} is error message {
  print "failed: ${message}"
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
  print "name: ${name}"
}
```