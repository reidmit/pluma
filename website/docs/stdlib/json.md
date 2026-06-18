# JSON

`std/json` reads and writes JSON: `parse` turns JSON text into a value you can
pick apart, and `stringify` turns a value back into text. It's the module you
reach for at the edge of a program (a reply from an API, a config file, a
request body), wherever data arrives as text and needs to become something typed.

## Parsing never crashes

JSON from outside your program is untrusted: it might be malformed, truncated, or
not JSON at all. So `parse` doesn't throw or halt: it hands back a
[`result`](/docs/reference/errors), `ok` with the parsed value or `err` with
where it gave up:

```pluma
use std/json

json.parse "42"      # => ok (value.int 42)
json.parse "{ oops"  # => err {line: 1, col: 3, message: "..."}
```

That `err` carries a `{line, col, message}` pointing at the problem, so a bad
payload is just a value you handle (`??` for a fallback, `try` to bail out), and
never a surprise that escapes. This is the whole posture of the module: the
text-to-value boundary is the one place you check, and everything past it is
ordinary typed Pluma.

## A parsed value is one of seven shapes

`parse` produces a `value`, an enum covering exactly the shapes JSON allows:

```pluma
enum value {
	null
	bool bool
	int int
	float float
	string string
	array (list value)
	object (dict string value)
}
```

An array is a `list` of values; an object is a [`dict`](/docs/stdlib/dict-set)
from string keys to values, the same standard collections from the rest of the
library, so once you're past the parse you walk JSON with tools you already know.

Notice numbers split into `int` and `float`. JSON has one number type, but Pluma
keeps whole numbers and decimals apart so a large integer doesn't lose precision
by being forced to a float. When you don't care which you got, `get-number` takes
either and gives you a float.

## Pulling values out

You could match a `value` with `when`, but the `get-*` helpers are quicker. Each
returns an [`option`](/docs/reference/errors): `some` when the value is the shape
you asked for, `none` otherwise, so a wrong type or a missing key is handled the
same gentle way as everywhere else.

```pluma
use std/json

json.get-string (json.value.string "hi")   # => some "hi"
json.get-int (json.value.int 42)            # => some 42
json.get-int (json.value.string "x")        # => none   (not an int)
```

`get-field` looks a key up on an object, and the rest pull a plain value out of a
leaf: `get-string`, `get-int`, `get-float`, `get-number`, `get-bool`,
`get-array`, `get-object`.

### A worked extraction

Reading one field out of a parsed object chains these together. `try` handles the
parse failure, and `??` supplies a stand-in if the field is absent before
`get-string` checks its type:

```pluma
use std/json

def name-of :: fun string -> option string = fun text {
	when json.parse text is ok doc {
		json.get-string ((json.get-field doc "name") ?? json.value.null)
	} else {
		none
	}
}

name-of "{\"name\": \"Ada\", \"age\": 36}"   # => some "Ada"
name-of "{ oops"                              # => none
```

Each step narrows untrusted text toward a typed `string`, and any misstep along
the way (bad JSON, no `name`, a `name` that isn't a string) lands on `none`
rather than blowing up.

## Building and writing JSON

You build a `value` the same way you'd build any enum, reaching its variants
through `value`:

```pluma
use std/json

let payload = json.value.array [json.value.int 1, json.value.int 2]
json.stringify payload   # => "[1,2]"
```

`stringify` produces compact text for sending over the wire; `stringify-pretty`
lays it out with a two-space indent and one item per line, which is what you want
for a config file or debug output.

## See also

- **[Dictionaries and sets](/docs/stdlib/dict-set)**: a JSON object comes out as
  a `dict string value`, walked with `std/dict`.
- **[Errors and missing values](/docs/reference/errors)**: the `result` from
  `parse` and the `option`s from the `get-*` helpers.
