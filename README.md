# pluma

A fun, friendly, statically-typed programming language.

**a work-in-progress...** (this readme will be updated if/when it's ready for people to try out)

to run while developing:
`./bin/run run`
`./bin/run build path/to/main.pa`

to test:
`./bin/test`

to build in release mode:
`./bin/build_release`

## examples

The syntax shown here may not be supported yet, or it may be slightly out of date, but these examples should give you an idea of what the language looks like:

```pluma
let name = "Reid"
let str = "hello $(name)!"

str
  . replace "hello" with "what's up"
  . replace "!" with "?"
  . uppercase ()
  . then { print $0 }
```

```pluma
struct Person (
  name :: String,
  age :: Int
)

def Person . greeting () -> String {
  p => "hi there, $(p.name)"
}

let p = Person ("Reid", 26)

print (person . greeting ())
```

```pluma
enum Color
  | Red
  | Green
  | Blue

def randomColor() -> Color =
  \():
    randomIntBetween 1 and 3 | match:
      case 1: Red
      case 2: Green
      case 3: Blue

let c = randomColor()

c | match:
  case Red(): print "it's red!"
  case Green(): print "it's green!"
  case Blue(): print "it's blue!"
  case _: print "???"

let c = randomColorFormat()

c | match:
  case RGB(r, g, b):
    print "rgb($(r), $(g), $(b))"
  case HSL(h, s, l):
    print "hsl($(h), $(s), $(l))"
  case Hex(val):
    print "#$(val)"
  case _:
    print "???"
```
