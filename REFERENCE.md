# Pluma language reference

## Basic types

## Tuples

Tuples are ordered, heterogenous collections of values.

```pluma
let tuple = (1, "hello")

# can always be accessed by position:
print (tuple.0)
```

Tuple elements can always be accessed by position, starting with 0. Accessing a position that doesn't exist (e.g. `("a", "b").4`) is a compile-time error, since the size of tuples is always fixed and known at compile time.

Tuple elements may also be given labels for convenience or readability.

```pluma
let tuple = (a: 1, b: "hello")

# can now be accessed by label:
print (tuple.b)

# can still be accessed by position:
print (tuple.0)
```

When a tuple has labels, the labels are part of its type.

```pluma
# Type signature specifies labels:
def labeledTuple(a: Int, b: Int) { (a, b) => a + b }

# Still valid, since (1, 2) is convertible to (a: Int, b: Int).
labeledTuple(1, 2)

# Type signature specifies no labels:
def unlabeledTuple(Int, Int) { (a, b) => a + b }

# Still valid, since (a: 1, b: 2) is convertible to (Int, Int)
unlabeledTuple(a: 1, b: 2)
```

Tuples may be partially labeled.

```pluma
let tup = (10, 20, a: "wow", b: "cool")

print (tup.0)
print (tup.1)
print (tup.a == tup.2)
print (tup.b == tup.3)
```

### Empty tuple

The empty tuple is a special case: `()`. It is often used to mean "nothing" or "no value". It has no fields or methods.

```pluma
# takes (), and also implicitly returns ()
def hello() {
  print "hello"
}

let result = hello() # result is ()
```

## Blocks, functions, and methods

Each of these **always takes exactly one argument**. However, with tuples (and the empty tuple), you can pass multiple values (or no values).

### Blocks

Blocks can appear anywhere, at any level. They cannot be exported from a module, even if defined at the top level.

Blocks usually have their types inferred from usage. Type assertions can be used to explicitly mark/assert that a block has a certain type.

```pluma
# empty arg:
let emptyArg = { print "hello" }
emptyArg()

# empty arg, explicit:
let emptyArg = { () => print "hello" }
emptyArg()

# simple arg:
let simpleArg = { print $0 }
oneArg "hello"

# simple arg, explicit:
let oneArg = { a => print a }
oneArg "hello"

# tuple arg:
let tupleArg = { print $0 + $1 }
tupleArg (1, 2)

# tuple arg, explicit:
let tupleArg = { (a, b) => print a + b }
tupleArg (a, b)
```

### Functions

Functions must be defined at the top level. They can be exported from a module, if they are public.

Functions must have a full, correct type signature.

```pluma
# empty arg:
def emptyArg() { print "hello" }
emptyArg()

# empty arg, explicit:
def emptyArg() { () => print "hello" }
emptyArg()

# simple arg:
def simpleArg String { print $0 }
oneArg "hello"

# simple arg, explicit:
def oneArg String { a => print a }
oneArg "hello"

# tuple arg:
def tupleArg (Int, Int) { print $0 + $1 }
tupleArg (1, 2)

# tuple arg, explicit:
def tupleArg (Int, Int) { (a, b) => print a + b }
tupleArg (1, 2)
```

Functions may have multi-part names. They still only take one argument; each part's arguments are collected into a tuple.

```pluma
# multi-part, tuple arg:
def tupleArg Int and Int { (a, b) => print a + b }
tupleArg 1 and 2

# ...is roughly equivalent to:
def tupleArg_and_ (Int, Int) { (a, b) => print a + b }
tupleArg_and_ (1, 2)
```

### Methods

Methods follow similar rules to functions. They must appear at the top level, they can be exported, and they can have multi-part names. The big difference is that **methods have receivers**. They are defined on a type, and must be called on values of that type.

The receiver is passed into the block in a tuple with the rest of the passed values. The receiver is always the first element.

```pluma
let p = Person(name: "Reid")

# self + empty arg:
def Person.emptyArg() { print "hello" }
p.emptyArg()

# self + empty arg, explicit:
def Person.emptyArg() { (self, ()) => print "hello" }
p.emptyArg()

# self + simple arg:
def Person.simpleArg String { print $1 }
p.oneArg "hello"

# self + simple arg, explicit:
def Person.oneArg String { (self, a) => print a }
p.oneArg "hello"

# self + tuple arg:
def Person.tupleArg (Int, Int) { print $1 + $2 }
p.tupleArg (1, 2)

# self + tuple arg, explicit:
def Person.tupleArg (Int, Int) { (self, a, b) => print a + b }
p.tupleArg (1, 2)

# self + multi-part, tuple arg:
def Person.tupleArg Int and Int { (self, a, b) => print a + b }
p.tupleArg 1 and 2

# an interesting case occurs when a method takes labeled tuple args:
def Person.namedArg (a: Int, b: Int) {
  (self, a, b) => print a + b
}
```

## Packages, modules, and export visibility

This is how your organize and share your code across different files, directories, and projects.

### Packages

A package is like a "project": a directory containing modules.

### Modules

A module is a file. It is identified by its file path relative to your
