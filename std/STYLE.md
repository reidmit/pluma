# Documentation style guide

These notes govern all the prose a Pluma user reads: the `#` doc
comments on stdlib `def`s and types (`std/*.pa`) and the in-app pages of
the website (`website/*.pa` — the tour, reference, and Get-started
pages). One voice across both surfaces. `std/list.pa` is the reference
module for comments; `website/reference.pa` for page prose. When in
doubt, match them.

## Who we're writing for

A working developer who's new to Pluma — fluent in Python, JavaScript,
or Go, comfortable with functions, types, and the rough idea that some
operations cost more than others. What they *don't* have is our
vocabulary or a functional-programming background. Write for a smart
peer who just hasn't met these words yet, not for a novice.

So:

- **No FP-theory jargon.** Never write "monad", "functor", "thunk",
  "partial application", "structural equality", "vacuously true",
  "instance" (typeclass sense), "desugars", "left fold", or
  "polymorphic". Explain the idea in plain words instead. ("Compares by
  value", not "structural equality"; "an empty list counts as true",
  not "vacuously true".)
- **Precision terms are fine — as asides.** Words like "partial",
  "primitive", and complexity notation (`O(1)`, `O(n^2)`, "amortized")
  carry real information this audience can use. Keep them out of the
  one-line summary, put them in an optional detail paragraph, and
  explain the term the first time it appears in a module.
- **No internal references.** The reader can't see our source tree.
  No file names, builtins, backend internals, or commit history. Point
  at another module by its import path: `std/option`, `std/bytes`.
- **Define a term the first time it earns its keep**, then reuse it.

## Voice: warm and plain-spoken

Friendly and direct, like a good README — not a stuffy manual, not a
stand-up comedian. The same register everywhere: the website tour is
not "more marketing" than the stdlib.

- Address the reader as "you" when it helps.
- Explain the *why*, not just the *what*, when it's short and useful.
- Prefer short, concrete sentences over precise-but-dense ones.
- Skip the jokes. Warmth comes from clarity, not gags.
- Be consistent. Same tone in every module and every page.

## Length: as short as the idea allows

Default to a one-sentence summary, one short detail paragraph, and an
example. That's enough for almost everything.

- Earn every extra paragraph. If the summary and example already say
  it, don't restate it in prose.
- Genuine footguns (in-place mutation, aliasing, partiality) get a
  single clear sentence of warning — not a multi-paragraph essay.
- When you catch yourself writing a third paragraph, ask whether a
  reader scanning a hover panel will read it. Usually: cut it.

## Coverage: document everything public

Every `public def` and every `public` type (enum, alias, opaque type)
gets a doc comment. No exceptions — if it's exported, a user can see it
in autocomplete and hovers, so it needs at least a one-line summary.
Private defs are documented only when the comment helps the next person
maintaining the module.

A type's comment goes on the line(s) directly above its declaration,
same as a `def`. Explain what the type represents and, when it's an
enum, what the variants mean.

## Structure of a stdlib comment

Each documented `def` gets:

1. **A one-sentence summary**, present tense — what it returns or does.
   Stands on its own; it's the autocomplete/one-line listing.
2. **An optional detail paragraph** — the edge case, the *why*, the one
   caveat. Keep it to one paragraph where you can.
3. **An example block**, indented under a blank `#` line.

```
# One-sentence summary that reads on its own.
#
# Optional one-paragraph detail: the edge case or why this exists.
#
#     module.name arg1 arg2   # => result
```

**Wrap comment prose at roughly 80–90 columns.** Comfortable line length
for reading in an editor; hover panels and the docs site reflow anyway.

### Module header

The first comment in the file introduces the module: what the core type
*is*, how to write a literal, conventions shared across the module, and
any vocabulary the function comments lean on. A few short paragraphs.

## Examples (stdlib)

Every function gets at least one example — the fastest way in for this
audience, and a promise we keep correct.

- Format: the call, a few spaces, then `# =>` and the result. This
  keeps the block valid Pluma source. One space after `=>`; align the
  `# =>` column across a block when it's easy.
- Show two or three lines when a contrast teaches something (a normal
  case plus an empty/edge case).
- **Write results in source notation** — the way you'd type the value,
  not the way `io.print` renders it. Strings keep their quotes;
  options/results use their constructors (`some 1`, `none`, `ok 42`,
  `err "bad input"`).
- **Every example must actually evaluate to the result shown.** Run it
  before committing. For long/brittle errors, describe the *shape*
  (`err {line, col, message}`) rather than inventing a string.
- **Non-deterministic results don't get a `# =>`.** Random values, the
  clock, env vars, file contents — use a plain `#` describing the
  outcome (`random.int ()   # a different number each call`).
- **Qualify names the way a caller writes them**: `list.map`,
  `json.value.int` — copy-pasteable after importing.
- **Families of near-identical wrappers share one example.** When a
  module exposes a large set of trivial wrappers that all behave the
  same way (HTML element helpers, CSS property shorthands, comparison
  operators), each still gets a one-line summary, but one or two
  representative examples in the module header cover the whole family.
  Don't repeat an example block on every member.

## Website prose

Same audience, same voice. The pages teach sequentially, so:

- Build from the ground up; each idea may assume the ones above it.
- Introduce a keyword or symbol in code, then explain it in prose — the
  reference's inline-code-then-sentence pattern.
- Examples are runnable Pluma. Show output as a trailing `#` comment the
  same way the stdlib does.
- A page may use the precision-aside rule too: a complexity or
  partiality note belongs in a `note` aside, not the main flow.

## Quick before/after

Before (accurate, but written for an insider):

```
# Left fold: `fold xs init f` reduces the list by applying
# `f acc element` left-to-right, threading `acc` through.
```

After (same facts, for our reader, one paragraph, worked example):

```
# Boils the whole list down to a single value, combining elements
# one at a time from left to right.
#
# You hand it the list, a starting value, and a function `f acc
# element`. Pluma walks front to back, calling `f` with the running
# total and the next element; whatever `f` returns becomes the new
# total. The final total is the answer.
#
#     list.fold [1, 2, 3, 4] 0 (fun acc n { acc + n })   # => 10
```
