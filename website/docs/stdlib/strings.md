# Working with strings

The [tour](/docs/tour/strings) covered string literals, interpolation, and
joining text with `++`. This page is about the `std/string` module — the library
of functions for measuring, slicing, searching, and reshaping text once you have
it.

Two things hold throughout. Strings never change in place: every function here
returns a *new* string and leaves the original alone. And positions and lengths
count **characters**, not bytes — an accented letter or an emoji is one character
even though it takes several bytes to store.

```pluma
use std/string

string.length "café"   # => 4   (four characters)
```

That character-based counting is what you almost always want. When you genuinely
need the raw bytes — writing to a socket, hashing — `string.to-bytes` hands them
over, and `string.byte-length` counts them instead.

## Measuring and indexing

```pluma
use std/string

string.length "hello"        # => 5
string.is-empty ""           # => true
string.char-at "hello" 1     # => some "e"
string.char-at "hello" 99    # => none   (out of range)
```

`char-at` returns an [`option`](/docs/reference/errors) because the index might
be past the end — there's no way to read a character that isn't there, so you
handle the `none` instead of risking a crash. `string.chars` explodes a string
into its characters as a list, which is handy when you want to walk text with
`list.map` or `list.filter`:

```pluma
string.chars "héllo"   # => ["h", "é", "l", "l", "o"]
```

## Slicing

`slice` takes a start and an end position (the end is not included), and `drop`
removes a number of characters from the front:

```pluma
use std/string

string.slice "hello" 1 4   # => "ell"
string.drop "hello" 2      # => "llo"
```

## Changing case and trimming

```pluma
use std/string

string.to-upper "Hello, World"   # => "HELLO, WORLD"
string.to-lower "Hello, World"   # => "hello, world"
string.trim "  hi there  "       # => "hi there"
```

`trim` removes whitespace from both ends — the usual first step on a line of user
input or a field read from a file.

## Searching

These all answer a yes/no question about whether one string appears in another:

```pluma
use std/string

string.contains "hello world" "o w"   # => true
string.starts-with "hello" "he"       # => true
string.ends-with "hello" "lo"         # => true
```

## Splitting and joining

`split` breaks a string on a separator into a list of pieces, and `join` is its
inverse — it glues a list of strings together with a separator between them.
They're the workhorses for line-oriented and delimited text:

```pluma
use std/string

string.split "a,b,c" ","            # => ["a", "b", "c"]
string.split "a,,b" ","             # => ["a", "", "b"]   (empty piece kept)
string.join ["a", "b", "c"] ", "    # => "a, b, c"
```

A consecutive separator leaves an empty piece rather than swallowing it, so
splitting and rejoining round-trips faithfully. `replace` swaps every occurrence
of one substring for another:

```pluma
string.replace "a-b-c" "-" "+"   # => "a+b+c"
```

## Parsing numbers

Turning text into a number can fail — `"42"` is a number, `"oops"` isn't — so
`to-int` and `to-float` return a [`result`](/docs/reference/errors), the same way
JSON parsing does:

```pluma
use std/string

string.to-int "42"      # => ok 42
string.to-int "3.5"     # => err "invalid digit found in string"
string.to-float "3.14"  # => ok 3.14
```

Handle the `err` with `??` for a default or `try` to propagate it, and a
malformed field never escapes as a surprise.

## See also

- **[Strings and text](/docs/tour/strings)** — literals, `$(...)` interpolation,
  and triple-quoted multi-line text.
- **[Bytes](/docs/reference/bytes)** — the raw-byte view, via `to-bytes`.
