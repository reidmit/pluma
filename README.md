# pluma

A fun, friendly, statically-typed programming language.

**a work-in-progress...** (this readme will be updated if/when it's ready for people to try out)

to run while developing:
`./scripts/run`
`./scripts/run build path/to/main.pa`

to test:
`./scripts/test`

to build in release mode:
`./scripts/build`

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

def randomColor() -> Color {
  match randomIntBetween 1 and 3
  | 1 => Red
  | 2 => Green
  | 3 => Blue
}

let c = randomColor()

match c
| Red => print "it's red!"
| Green => print "it's green!"
| Blue => print "it's blue!"
```
