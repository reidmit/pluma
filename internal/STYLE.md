# Writing & documentation style guide

These notes govern all the prose a Pluma user reads. There are two
surfaces, and they share one voice:

- **Doc comments**: the `#` comments on stdlib `def`s and types
  (`std/*.pa`), surfaced in autocomplete, hovers, and the generated
  stdlib pages.
- **The docs website**: the in-app Markdown pages served at
  pluma.fun/docs (`website/data/docs/**/*.md`): the tour, the reference, the
  guides, and the standard-library walkthroughs.

One voice across both. The website tour is not "more marketing" than the
stdlib, and a hover panel is not terser-but-colder than a guide.
`std/list.pa` is the reference module for comments; `website/data/docs/tour/`
for page prose. When in doubt, match them.

The guide is organized as: who and how we write (the parts that apply
everywhere), then the mechanics of each surface.

---

## Part 1: Voice (applies everywhere)

### Who we're writing for

A working developer who's new to Pluma: fluent in Python, JavaScript,
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
- **Precision terms are fine, as asides.** Words like "partial",
  "primitive", and complexity notation (`O(1)`, `O(n^2)`, "amortized")
  carry real information this audience can use. Keep them out of the
  one-line summary, put them in an optional detail paragraph, and
  explain the term the first time it appears in a module.
- **No internal references.** The reader can't see our source tree.
  No file names, builtins, backend internals, or commit history. Point
  at another module by its import path: `std/option`, `std/bytes`.
- **Define a term the first time you genuinely need it**, then reuse it.

### Warm and plain-spoken

Friendly and direct, like a good README: not a stuffy manual, not a
stand-up comedian. The same register everywhere.

- Address the reader as "you" when it helps.
- Explain the *why*, not just the *what*, when it's short and useful.
- Prefer short, concrete sentences over precise-but-dense ones.
- Skip the jokes. Warmth comes from clarity, not gags.
- Be consistent. Same tone in every module and every page.

### As short as the idea allows

Default to a one-sentence summary, one short detail paragraph, and an
example. That's enough for almost everything.

- Cut every extra paragraph that doesn't pay for itself. If the summary
  and example already say it, don't restate it in prose.
- Genuine footguns (in-place mutation, aliasing, partiality) get a
  single clear sentence of warning, not a multi-paragraph essay.
- When you catch yourself writing a third paragraph, ask whether a
  reader scanning a hover panel or skimming a page will read it.
  Usually: cut it.

### Banned words and idioms

A hard list. These never appear in either surface, no exceptions. Most
are tells of machine-generated prose, and cutting them keeps the writing
sounding like a person wrote it. Add to the list whenever a new cliché
starts creeping in.

- **Em-dashes (the `—` character).** The clearest tell of LLM-generated
  text. Use a colon, a comma, a pair of parentheses, or two separate
  sentences instead. (A hyphen inside an identifier like `to-string` is
  fine. An en-dash in a number range like `80–90` is fine. The ban is
  the em-dash specifically.)
- **"load-bearing."** Never describe anything as load-bearing.
- **"earns its keep" / "earn its keep" / "earns their keep"** and any
  other variant. Say "is worth it", or just state the point plainly.

### House conventions

These keep the two surfaces consistent with each other:

- **Write identifiers exactly as they appear in source, in backticks.**
  Pluma keeps the lowercase, hyphenated spelling: `to-string`,
  `list.fold`, `rpc-client`.
- **Tabs, not spaces, in every code sample.** The language is
  tab-indented and the samples must round-trip through the formatter.
- **Output is a trailing comment, in source notation.** `# => 10` for a
  value; a plain `#` describing the outcome when it's non-deterministic.
  (Details under each surface below.)
- **Sentence case for headings and titles** ("Get started", not "Get
  Started").
- **One space after `# =>`.** Strings keep their quotes; options and
  results use their constructors (`some 1`, `none`, `ok 42`, `err "…"`).

---

## Part 2: Doc comments (`std/*.pa`)

### Coverage: document everything public

Every `public def` and every `public` type (enum, alias, opaque type)
gets a doc comment. No exceptions: if it's exported, a user can see it
in autocomplete and hovers, so it needs at least a one-line summary.
Private defs are documented only when the comment helps the next person
maintaining the module.

A type's comment goes on the line(s) directly above its declaration,
same as a `def`. Explain what the type represents and, when it's an
enum, what the variants mean.

### Structure of a comment

Each documented `def` gets:

1. **A one-sentence summary**, present tense: what it returns or does.
   Stands on its own; it's the autocomplete/one-line listing.
2. **An optional detail paragraph**: the edge case, the *why*, the one
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

### Examples

Every function gets at least one example: the fastest way in for this
audience, and a promise we keep correct.

- Format: the call, a few spaces, then `# =>` and the result. This
  keeps the block valid Pluma source. Align the `# =>` column across a
  block when it's easy.
- Show two or three lines when a contrast teaches something (a normal
  case plus an empty/edge case).
- **Write results in source notation**: the way you'd type the value,
  not the way `io.print` renders it.
- **Every example must actually evaluate to the result shown.** Run it
  before committing. For long/brittle errors, describe the *shape*
  (`err {line, col, message}`) rather than inventing a string.
- **Non-deterministic results don't get a `# =>`.** Random values, the
  clock, env vars, file contents: use a plain `#` describing the
  outcome (`random.int ()   # a different number each call`).
- **Qualify names the way a caller writes them**: `list.map`,
  `json.value.int`, copy-pasteable after importing.
- **Families of near-identical wrappers share one example.** When a
  module exposes a large set of trivial wrappers that all behave the
  same way (HTML element helpers, CSS property shorthands, comparison
  operators), each still gets a one-line summary, but one or two
  representative examples in the module header cover the whole family.
  Don't repeat an example block on every member.

---

## Part 3: The docs website (`website/data/docs/**`)

The docs are Markdown files, rendered by Pluma itself: `std/markdown`
parses the file to an AST and `std/view` renders it, the same renderer
the rest of the site uses. There is no separate stylesheet and no build
step: the server reads the `.md` at request time, so editing a file and
refreshing shows the change.

### Anatomy of a page

A page is one `.md` file under `website/data/docs/<group>/<slug>.md`. Its
structure:

- **One `#` H1** at the top: the page title. One per page.
- **An intro paragraph** right after it: what this page covers and, for
  tour pages, what it assumes you've already read. No "## Introduction"
  heading; just lead with the prose.
- **`##` sections** for the body. Each `##` and `###` heading
  automatically gets an `id` anchor derived from its text, so the "On
  this page" sidebar and `#fragment` links work for free; you don't
  write the anchors.

Keep headings in sentence case and short; they double as the table of
contents.

### Adding a page to the site

A page only appears in the nav once it's registered. In
`website/docs.pa`, add one entry to the right group in `groups`:

```
{slug: "tour/closures", title: "Closures", file: "tour/closures"}
```

- `slug` is the path after `/docs` (so this renders at
  `/docs/tour/closures`; `""` is the index).
- `title` is the sidebar label.
- `file` is the basename of the `.md` under `website/data/docs/`.

Then drop `website/data/docs/tour/closures.md` next to its siblings. A new
section in the sidebar is a new `doc-group` record. Order in the file is
order in the sidebar: put pages in teaching order, not alphabetical.

### Which section am I writing?

The four groups have distinct jobs; match the one you're in.

- **Tour** (`tour/*`): teaches the language front-to-back. Each page
  may assume every page above it. Build from the ground up; introduce
  one idea at a time and link forward to where it's covered in depth.
- **Reference** (`reference/*`): look-things-up pages: operators, error
  codes, build pipeline. Assume the tour. Complete and precise over
  gentle; a reader arrives here knowing what they want.
- **Guides** (`guides/*`): one small, complete program per page (a CLI
  script, a server, a fullstack app). Show the whole thing working.
- **Standard library** (`stdlib/*`): task-oriented walkthroughs of a
  module ("Working with lists"). These complement, not duplicate, the
  per-`def` doc comments: the comment is the spec for one function, the
  page is the guided tour of the module.

### Code blocks

Fence Pluma with the `pluma` info string so it's recognized as a sample:

````
```pluma
def main = fun {
	print "hello, world"
}
```
````

- **Tabs, not spaces.** Samples must survive the formatter.
- **Every sample is runnable Pluma**, or a clearly-labeled fragment. If
  it's a whole program, it should compile and run.
- **Show output as a trailing `#` comment**, the same notation the
  stdlib uses: `print x   # hello` or `# => 10`. Don't paste a separate
  "Output:" block.
- Use a plain fence (no `pluma`) for shell commands, file trees, and
  diagnostic output: anything that isn't Pluma source.

### Callouts

Use a fenced-div container for an aside that should stand out. The
opener is `:::`, a bare `:::` closes it, and the spec picks the element
and classes:

```
::: aside .callout
**New to Pluma?** Start with [Get started](/docs/start) first.
:::
```

That renders `<aside class="callout"> … </aside>`. Reach for one for a
genuine "heads up" or a pointer to a prerequisite, not for ordinary
emphasis. One or two per page at most; a page full of callouts has none.

### Links

- **Cross-link liberally.** Root-relative paths to other doc pages:
  `[Modules](/docs/tour/modules)`, `[the standard library](/docs/stdlib)`.
- When you introduce an idea that's covered in depth elsewhere, link to
  it rather than re-explaining; that's what lets each page stay short.
- Link the first, natural mention; don't link the same target five
  times in a paragraph.

### Tables and the rest

GFM pipe tables render: use them for genuinely tabular reference
material (error codes, operator precedence), not for layout. The
supported Markdown subset is bold/italic/code, links, images,
blockquotes, bulleted and numbered lists (one level of nesting), fenced
code, thematic breaks, and the callout containers above. Anything it
doesn't recognize passes through as literal text, so keep to the subset.

### Teaching order on a page

The pages teach sequentially, so within a page too:

- Introduce a keyword or symbol in a code sample, then explain it in
  prose, the inline-code-then-sentence pattern the reference uses.
- A complexity or partiality note belongs in a `::: aside` callout, not
  the main flow (the precision-aside rule, applied to pages).
- End a tour page with a one-line pointer to the next idea when it helps
  the reader keep moving.

---

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
