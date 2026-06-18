# Strings and text

Strings are written with double quotes. To drop a value into the middle of one,
use `$(...)` — whatever's inside the parentheses is evaluated and spliced in.

```pluma
let name = "Ada"
print "hello, $(name)"          # hello, Ada
```

Only `$(` starts interpolation. To write one literally — a shell snippet, say —
escape the dollar as `\$(`.

## Text is always text

A number isn't automatically a string. When you want to show one, you convert it
with `to-string`:

```pluma
let n = 42
print "the answer is $(to-string n)"   # the answer is 42
```

This is the same no-silent-conversion rule from the [basics](/docs/tour/basics)
page, applied to text: Pluma never guesses that you meant to turn a number into
text, so the `to-string` is always explicit. Join two strings with `++`:

```pluma
"foo" ++ "bar"     # => "foobar"
```

## Multi-line text

For longer text, use a triple-quoted string. The opening `"""` is followed by a
line break, and the indentation of the *closing* `"""` sets the left margin —
that much leading whitespace is stripped from every line, so the text lines up
with your code without the indentation leaking into the value.

```pluma
def page = """
	<ul>
		<li>$(name)</li>
	</ul>
	"""
```

Double quotes need no escaping inside a triple-quoted string, and `$(...)`
interpolation works just as it does in an ordinary string. It's the natural way
to write an HTML fragment, a SQL query, or any block of text that spans lines.

Next: [Lists, tuples, and records](/docs/tour/collections).
