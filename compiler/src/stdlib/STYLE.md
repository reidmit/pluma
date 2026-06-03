# Stdlib doc-comment style guide

These notes govern the `#` doc comments on `def`s in the standard
library (`compiler/src/stdlib/*.pa`). They are the comments users will
eventually read in editor hovers, go-to-definition, and the docs
website — so they're documentation, not internal notes.

`compiler/src/stdlib/list.pa` is the reference module. When in doubt,
match it.

## Who we're writing for

Picture a sharp high-schooler who builds real projects in Python,
JavaScript, or Go, but has never taken a CS theory class. They're smart
and curious; they just haven't met our vocabulary yet.

So:

- **No functional-programming jargon.** Never write "monad", "functor",
  "higher-order function", "thunk", "partial application", "structural
  equality", "vacuously true", "short-circuit" (as a noun), "instance"
  (in the typeclass sense), "desugars", "left fold", or "polymorphic".
  If a concept needs one of these, explain the idea in plain words
  instead. ("Compares by value" not "structural equality"; "an empty
  list counts as true" not "vacuously true".)
- **No internal references.** The reader can't see our source tree.
  Don't mention file names, builtins, backend internals, or commit
  history. To point at another stdlib module, name it as `core.option`,
  `core.bytes`, etc.
- **Define a term the first time it earns its keep**, then reuse it.
  `core.list` defines "predicate" once in the module header and then
  uses the word freely.

## Voice: warm and plain-spoken

Friendly and direct, like a good README — not a stuffy manual, not a
stand-up comedian.

- Address the reader as "you" when it helps ("you get back `none`",
  "you hand it three things").
- Explain the *why*, not just the *what*, when it's short and useful.
- Prefer short, concrete sentences over precise-but-dense ones.
- Skip the jokes. Warmth comes from clarity and the occasional plain
  aside, not from gags — they get stale and don't translate.
- Be consistent. Same tone in every module.

## Structure of a comment

Each `def` gets:

1. **A one-sentence summary**, present tense, describing what the
   function returns or does. This line stands on its own — it's what
   shows up in autocomplete and one-line listings.
2. **Optional detail paragraph(s)** — edge cases, the *why*, gotchas.
   Only if they add something the summary and example don't.
3. **An example block**, indented under a blank `#` line.

````
# One-sentence summary that reads on its own.
#
# Optional detail: the edge case or the reason this exists.
#
#     module.name arg1 arg2   # => result
````

Wrap comment prose at roughly 64 columns so it stays readable in narrow
hover panels.

### Module header

The first comment in the file (before the first `def`) introduces the
module. This is where you teach the cross-cutting concepts a newcomer
needs before any single function makes sense: what the core type *is*,
how to write a literal, conventions shared across the module (e.g. "a
lookup that might fail returns an `option`"), and any vocabulary the
function comments will lean on. Keep it to a few short paragraphs.

## Examples are required

Every function gets at least one example. They're the fastest way for
this audience to understand a function, and they double as a promise we
keep correct.

- Format: the call, a few spaces, then `# =>` and the result. Writing
  the result as a `#` comment means the example block stays valid Pluma
  source — you could paste it into a program and it would parse, with
  the result reading as "evaluates to". (`=>` also keeps it distinct
  from the `->` in type signatures.) Use one space after `=>`, and
  align the `# =>` column across a block of examples when it's easy.
- Show two or three lines when a contrast teaches something: a normal
  case plus an empty/edge case (`list.head [1,2,3]` *and* `list.head []`).
- **Write results in source notation — the way you'd type the value —
  not the way `io.print` happens to render it.** This matters:
  - Strings keep their quotes: `["1", "2", "3"]`, never `[1, 2, 3]`.
    (Printing a string list drops the quotes, which would make a list
    of strings look identical to a list of numbers.)
  - Options and results use their constructors: `some 1`, `none`,
    `ok 42`, `err "bad input"` — not the printer's `option.some 1`.
- **Every example must actually evaluate to the result shown.** Verify
  by running it before committing — drop the calls into a scratch
  `main.pa` and `io.print` them, or add them to the module's
  `*.test.pa`. Don't eyeball outputs. This includes error messages: show
  the real `err` text (run it), and if the real text is long or brittle,
  describe the error's *shape* instead (e.g. `err {line, col, message}`)
  rather than inventing a string.
- **When the result isn't deterministic, don't fake a `# =>`.** For
  random values, the current time, environment variables, or file
  contents — anything that varies by run or machine — use a plain `#`
  comment describing the outcome instead (`random.int ()   # a different
  number each call`). Reserve `# =>` for things that always evaluate to
  exactly the shown value.
- **Qualify names the way a caller writes them.** Examples use
  `list.map`, `json.value.int`, `package.dep.simple` — the module-
  prefixed form a user types after importing — so they're copy-pasteable
  as-is.

## Quick before/after

Before (accurate, but written for an insider):

````
# Left fold: `fold xs init f` reduces the list by applying
# `f acc element` left-to-right, threading `acc` (starting at `init`)
# through.
````

After (same facts, for our reader, with a worked example):

````
# Boils the whole list down to a single value by combining the
# elements one at a time, left to right.
#
# You hand it three things: the list, a starting value, and a
# function `f acc element`. Pluma walks the list front to back, and
# at each element calls `f` with the running total so far (the
# "accumulator") and that element; whatever `f` returns becomes the
# new running total. When the elements run out, the final total is
# the answer.
#
#     list.fold [1, 2, 3, 4] 0 (fun acc n { acc + n })   # => 10
````
