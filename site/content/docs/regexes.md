+++
title = "Regexes"
description = "First-class regular expressions with a structured, whitespace-tolerant syntax — closer to a parsing DSL than to PCRE."
weight = 3
+++

Pluma's regex syntax is intentionally different from the dense punctuation of PCRE-style regexes. Patterns are built by composing *atoms* — string literals, named character classes, and anchors — with *combinators* for sequencing, alternation, grouping, and repetition. Whitespace between atoms is meaningless, so you can lay a pattern out across multiple lines like ordinary code.

```
let phone = /
    ^
    "(" digit{3} ")"
    " "
    digit{3} "-" digit{4}
    $
/
```

The result is a value of the primitive type `regex`, which can only be tested against a string via `regex.matches`.

## The literal

Regex literals are delimited by forward slashes:

```
let hello = /"hello"/
let yes-or-no = /"yes" | "no"/
```

An empty regex (`//` with no body) is a parse error.

## Atoms: string literals

Unlike most regex flavors, the building block of a Pluma regex is a **string literal** — not a single character. Anything that would be a string in source code is also a valid regex atom.

```
/"hello"/                       # matches the exact 6 characters
/"color: " "red"/               # two atoms, concatenated
/"\t\n"/                        # escapes work — matches tab+newline
/"reid's "/                     # punctuation needs no escaping inside ""
```

Because every literal is wrapped in quotes, regex metacharacters never need to be escaped at the regex level — escapes only apply to the *contents* of the string. There are no PCRE-style bracket character classes or `\d`-style shorthands; named atoms (next section) cover the common ones.

## Character classes

For matching "any character of some kind," Pluma uses **named atoms** instead of PCRE's `\d` / `\w` / `\s` shorthands or bracket sets like `[A-Z]`. The known names are:

| Name | Matches |
| - | - |
| `digit` | A single ASCII digit (`0`–`9`) |
| `letter` | A single ASCII letter (`A`–`Z`, `a`–`z`) |
| `word` | A single letter, digit, or `_` |
| `whitespace` | A single space, tab, newline, or carriage return |
| `any` | Any single character |

Each name is **one character wide** and composes with all the combinators below.

```
/digit/                         # one digit anywhere in the input
/letter+/                       # one or more letters
/word{3,}/                      # three or more word characters
/digit "-" digit/               # mixed with literal atoms
/letter (digit | "-")*/         # mixed with alternation in a group
```

Bare identifiers inside a regex that aren't on the table above are a compile error — there's no fallback to "treat the letters as a custom set" the way `[abc]` would in PCRE.

## Sequencing &amp; whitespace

Two atoms written one after another match in sequence. Whitespace between them is purely cosmetic — including line breaks. The following three definitions are identical:

```
let a = /"hello" "world"/

let b = /"hello"   "world"/

let c = /
    "hello"
    "world"
/
```

This is the main reason to reach for Pluma's regex over a string-based pattern: complex regexes can be laid out vertically, one atom per line, and remain readable.

## Alternation: `|`

`|` tries the left side first, then the right. At the top level it splits the whole pattern; inside a group it splits within the group.

```
let yes-or-no = /"yes" | "no"/

let primary-color = /"red" | "green" | "blue"/

let labeled = /"color: " ("red" | "green" | "blue")/
```

## Grouping: `(…)`

Parentheses group sub-patterns so quantifiers and alternation apply to the whole group. Groups are **non-capturing** — they only affect parsing, not the output of a match.

```
/("ab")+/                       # one-or-more of the sequence "ab"
/"x" ("y" | "z")? "w"/          # optional alternation in the middle
```

Empty groups (`()`) are a parse error.

## Named captures: `<name: …>`

To capture a sub-pattern by name, use the angle-bracket form:

```
let timestamp = /
    <year:  "2024" | "2025" | "2026">
    "-"
    <month: "01" | "02" | "03" | "04" | "05" | "06"
           | "07" | "08" | "09" | "10" | "11" | "12">
/
```

Capture names must be identifiers. Captures are how you'll extract structure from a match once richer match APIs land — for now (see [the API](#the-regex-type-and-core-regex) below) only boolean matching is exposed, so captures parse and compile but aren't yet readable from user code.

## Quantifiers

Quantifiers apply to the atom or group immediately to their left.

| Syntax | Meaning |
| - | - |
| `x?` | Zero or one |
| `x*` | Zero or more |
| `x+` | One or more |
| `x{n}` | Exactly `n` |
| `x{n,}` | At least `n` |
| `x{,m}` | At most `m` (zero up to `m`) |
| `x{n,m}` | Between `n` and `m` inclusive |

Examples:

```
let two-to-four-a    = /"a"{2,4}/
let at-least-one-b   = /"b"+/
let optional-c       = /"c"?/
let any-many-spaces  = /" "*/
let exact-three      = /"!"{3}/

let opt-prefix = /("yes, " | "no, ")? "thanks"/   # optional alternation group
```

`{n,m}` with `n > m` is rejected at parse time.

## Anchors

Anchors are **zero-width** atoms — they don't consume any input, they assert a position. There are three:

| Symbol | Matches at |
| - | - |
| `^` | The start of the input |
| `$` | The end of the input |
| `%` | A word boundary (the position between a `word` character and a non-`word` character, including the very start and end of the input) |

```
/^ "hello"/                     # input must start with "hello"
/".pa" $/                       # input must end with ".pa"
/^ "yes" $/                     # input must be exactly "yes"
/^ digit+ $/                    # input must be only digits

/% "cat" %/                     # match "cat" as a whole word — not "category"
/% digit/                       # any word that starts with a digit
```

A quantifier on an anchor (`^?`, `$*`, `%+`, etc.) is a parse error — repeating a position assertion doesn't make sense.

The `%` mnemonic: a line separating two dots, like a boundary between two words.

## The `regex` type and `core.regex`

The literal `/…/` produces a value of the primitive type `regex`. Compilation happens once, at the regex's definition site — the value carries the compiled matcher.

The standard library exposes a single operation today:

```
use core.regex

regex.matches :: fun regex string -> bool
```

Worked example:

```
use core.regex

def hello = /"hello"/

def main = fun {
    print (regex.matches hello "hello, world!")    # true
    print (regex.matches hello "goodbye, world!")  # false
    print (regex.matches hello "hello")            # true
    print (regex.matches hello "")                 # false
}
```

You can also bind the module to a shorter name:

```
use core.regex as re

def is-yes = /"yes"/

def main = fun {
    print (re.matches is-yes "yes!")               # true
    print (re.matches is-yes "no thanks")          # false
}
```

{% note() %}
`regex.matches` tests whether the regex matches *anywhere* in the input — there's no implicit anchoring. Use the [anchors](#anchors) `^` and `$` to pin the match to the start or end of the string.
{% end %}

## How it differs from PCRE

| PCRE | Pluma |
| - | - |
| `hello` | `"hello"` |
| `a\|b` | `"a" \| "b"` |
| `\d`, `\w`, `\s`, `.` | `digit`, `word`, `whitespace`, `any` (bare names, not escapes) |
| `[abc]`, `[A-Z]` | Not available — use alternation or a named class |
| `^`, `$` | `^`, `$` (same) |
| `\b` | `%` (single glyph, no escape) |
| `(?:ab)+` | `("a" "b")+` (all groups are non-capturing) |
| `(?P<y>\d{4})` | `<y: digit{4}>` |
| Whitespace is significant by default | Whitespace between atoms is always ignored |
| Metacharacters need escaping | No metacharacters at the regex level — everything literal lives in `"…"` |
