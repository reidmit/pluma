# Regular expressions

Pluma's regexes are a structured, whitespace-tolerant DSL, closer to a small
parser than to PCRE. Patterns compose *atoms* (string literals, named classes,
anchors) with combinators for sequence, alternation, grouping, and repetition.
Whitespace between atoms is meaningless, so a pattern can lay out across lines:

```pluma
let phone = `
	^
	"(" digit{3} ")"
	" "
	digit{3} "-" digit{4}
	$
`
```

A literal is delimited by backticks and produces a value of the primitive type
`regex`, which you test and extract from with `std/regex`. An empty regex is a
parse error.

## Atoms and classes

Unlike most flavors, the building block is a *string literal*, not a single
character, so metacharacters never need escaping at the regex level. For "any
character of a kind," use a named class instead of `\d`-style shorthands:

| Name | Matches |
| --- | --- |
| `digit` | A single ASCII digit (0–9) |
| `letter` | A single ASCII letter (A–Z, a–z) |
| `word` | A single letter, digit, or _ |
| `whitespace` | A space, tab, newline, or carriage return |
| `any` | Any single character |

```pluma
`"hello"`               # the exact characters
`"color: " "red"`       # two atoms, in sequence
`digit "-" digit`       # a class mixed with literals
`letter (digit | "-")*`  # alternation inside a group
```

## Alternation, grouping, captures

`|` tries the left side first, then the right. Parentheses group a sub-pattern
(groups are non-capturing). To capture by name, use the angle-bracket form
`<name: ...>`:

```pluma
`"yes" | "no"`
`"color: " ("red" | "green" | "blue")`
`<key: letter+> "=" <val: digit+>`
```

## Quantifiers

A quantifier applies to the atom or group immediately to its left.

| Syntax | Meaning |
| --- | --- |
| `x?` | Zero or one |
| `x*` | Zero or more |
| `x+` | One or more |
| `x{n}` | Exactly n |
| `x{n,}` | At least n |
| `x{,m}` | At most m |
| `x{n,m}` | Between n and m, inclusive |

## Anchors

Anchors are zero-width: they assert a position rather than consume input. A
quantifier on an anchor is a parse error.

| Symbol | Asserts |
| --- | --- |
| `^` | The start of the input |
| `$` | The end of the input |
| `%` | A word boundary (between a word and non-word character) |

## Using std/regex

```pluma
use std/regex

regex.matches       :: fun regex string -> bool
regex.find          :: fun regex string -> option regex.match
regex.find-all      :: fun regex string -> list regex.match
regex.named-capture :: fun regex string string -> option string
regex.replace       :: fun regex string string -> string
regex.split         :: fun regex string -> list string
```

Every match surfaces as a record: the text, its byte offsets, and the named
captures that fired:

```pluma
alias regex.match {
	text   :: string,
	start  :: int,
	end    :: int,
	groups :: dict string string,
}
```

```pluma
use std/regex as re

def pair = `<key: letter+> "=" <val: digit+>`

def main = fun {
	when re.find pair "size=42" is some m {
		print m.text                     # size=42
	} is none {
		print "no match"
	}
}
```

::: aside .callout
**Coming from PCRE:** `\d \w \s .` become the bare names
`digit word whitespace any`; `\b` becomes `%`; all groups are non-capturing; and
`matches`/`find` are unanchored, so pin them with `^` and `$`.
:::
