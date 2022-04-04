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
let str = "hello " ++ name ++ "!"

str
  | replace "hello" with "what's up"
  | replace "!" with "?"
  | uppercase
  | then (print _)
```

```pluma
struct person (
  name :: string
  age :: int
)

def _ | greeting :: person -> string {
  p => "hi there, " ++ p.name
}

let p = person ("Reid", 27)

print (person | greeting)
```

```pluma
enum color { red, green, blue }

def random-color _ :: () -> color {
  random-int-between 1 and 3 | match {
    case 1 => red
    case 2 => green
    case 3 => blue
  }
}

let c = random-color ()

c | match {
  case red => print "it's red!"
  case green => print "it's green!"
  case blue => print "it's blue!"
  case _ => print "???"
}
```
