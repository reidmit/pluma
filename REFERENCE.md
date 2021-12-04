# Pluma language reference

## Basic types

### Integers

```pluma
let n = 47
let age = 27
```

### Floats

```pluma
let price = 19.99
let gpa = 4.0
```

### Booleans

Actually a built-in enum type!

```pluma
let t = True
let f = False
```

## Tuples

Tuples are ordered, fixed-length, heterogenous collections of values.

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
def labeledTuple (a: Int, b: Int) = \args:
  args.a + args.b

# Still valid, since (1, 2) is convertible to (a: Int, b: Int).
labeledTuple (1, 2)

# Type signature specifies no labels:
def unlabeledTuple (Int, Int) = \a:
  a.0 + a.1

# Still valid, since (a: 1, b: 2) is convertible to (Int, Int)
unlabeledTuple (a: 1, b: 2)
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
def hello () = \:
  print "hello"

let result = hello() # result is ()
```

## Lists

Lists are not fixed-length, but they must contain elements of a single type.

```pluma
let nums = [1, 2, 3]
let strings = ["hey", "there"]

nums[0] + nums[1] == nums[2]
```

## Dicts

Dicts are not fixed-size, but they must contain string keys and values of a single type.

```pluma
let dict = { "a": True, "b": False }
let people = { "jack": 47, "jill": 42 }

people["jack"] == 47
```

## Blocks, functions, and methods

Each of these **always takes exactly one argument**. However, with tuples (and the empty tuple), you can pass multiple values (or no values).

### Blocks

Blocks can appear anywhere, at any level. They cannot be exported from a module, even if defined at the top level.

Blocks usually have their types inferred from usage. Type assertions can be used to explicitly mark/assert that a block has a certain type.

```pluma
# empty arg:
let emptyArg = : print "hello"
emptyArg | call

# empty arg, explicit:
let emptyArg = \(): print "hello"
emptyArg | call ()

# empty arg, with line break:
let emptyArg = :
  print "hello"
emptyArg | call

# simple arg:
let simpleArg = \a: print a
oneArg | call "hello"

# simple arg, with line break:
let oneArg = \a:
  print a
oneArg | call "hello"

# unlabeled tuple arg:
let tupleArg = \tup: print (tup.0 + tup.1)
tupleArg | call (1, 2)

# labeled tuple arg:
let tupleArg = \tup: print (tup.a + tup.b)
tupleArg | call (a: 1, b: 2)

# unlabeled tuple arg, destructured:
let tupleArg = \(a, b): print a + b
tupleArg | call (a, b)

# takes no args, returns ()
let noop = :()
noop | call
noop | call ()

# with a type assertion
let withTypeAssertion :: (Int, Int) -> Int =
  \(a, b): a + b
```

### Functions

Functions must be defined at the top level. They will be exported from a module if they are public.

Functions must have a full, correct type signature.

```pluma
# empty arg:
def emptyArg () = :
  print "hello"
emptyArg()

# empty arg, explicit:
def emptyArg () = \():
  print "hello"
emptyArg()

# simple arg:
def simpleArg String = \s: print s
oneArg "hello"

# tuple arg:
def tupleArg (Int, Int) = \tup: print (tup.0 + tup.1)
tupleArg (1, 2)

# tuple arg, destructured:
def tupleArg (Int, Int) = \(a, b): print (a + b)
tupleArg (1, 2)
```

Functions may have multi-part names. They still only take one argument; each part's arguments are collected into a tuple.

```pluma
# multi-part, tuple arg:
def tupleArg Int and Int = \(a, b): print (a + b)
tupleArg 1 and 2

# ...is roughly equivalent to:
def tupleArg_and_ (Int, Int) = \(a, b): print (a + b)
tupleArg_and_ (1, 2)
```

### Methods

Methods follow similar rules to functions. They must appear at the top level, they can be exported, and they can have multi-part names. The big difference is that **methods have receivers**. They are defined on a type, and must be called on values of that type.

The receiver is passed into the block in a tuple with the rest of the passed values. The receiver is always the first element.

```pluma
let p = Person (name: "Reid")

# self + empty arg:
def Person | emptyArg () = :
  print "hello"
p | emptyArg()

# self + empty arg, explicit:
def Person | emptyArg () = \(self, ()):
  print "hello"
p | emptyArg()

# self + simple arg:
def Person | simpleArg String = \(self, arg):
  print arg
p | simpleArg "hello"

# self + simple arg, explicit:
def Person | oneArg String = \(self, a): print a
p | oneArg "hello"

# self + tuple arg:
def Person | tupleArg (Int, Int) = \(self, a, b): print a + b
p | tupleArg (1, 2)

# self + tuple arg, explicit:
def Person | tupleArg (Int, Int) = \(self, a, b): print a + b
p | tupleArg (1, 2)

# self + multi-part, tuple arg:
def Person | tupleArg Int and Int = \(self, a, b):
  print a + b
p | tupleArg 1 and 2

# an interesting case occurs when a method takes labeled tuple args:
def Person | namedArg (a: Int, b: Int) = \(self, a, b):
  print a + b
p | namedArg (a: 1, b: 2)
```

## Modules, packages, and export visibility

This section describes how to organize and share your code across different files, directories, and projects.

### Modules

A module is a file.

Imagine you have a file called `helpers/math.pa`:

```pluma
def add (Int, Int) -> Int = \arg:
  arg.0 + arg.1
```

And in another file, called `main.pa`:

```pluma
use @math helpers/math

let three = @math add (1, 2)
```

You could also import all exports into the common namespace:

```pluma
use helpers/math

let three = add (1, 2)
```

If you had multiple `use` statements, and each one declared a name `add`, you'd get a compile error due to the duplicate declarations. It's recommended to qualify your imports with `@qualifier`.

### Packages

A package is like a "project": a directory containing modules.

If a package is compiled directly as a binary, the compiler will look for a `main.pa` file in the directory to use as the entrypoint.

### Export visibility

```pluma
# All top-level defs are exported by default:
def somePublicDef() = : ()
def anotherPublicDef() = : ()

# But you can change the visibility of following defs with the `private`/`internal`
# keywords.

internal

# The following can only be accessed by modules within the same package (directory):
def thisIsInternal() = : ()

private

# The following can only be accessed within this module (file):
def thisIsPrivate() = : ()
```

Although you may repeat these keywords (e.g. have a default public section, then a `private`, then an `internal`, then another `private`), it's recommended to stick to the above example (all public defs first, then all `internal` if any, then all `private` if any).
