# Documentation

Welcome to the Pluma docs. Pluma is a small, statically-typed functional
language with full type inference and a fullstack framework that compiles to
WebAssembly — one language for the command line, the server, and the browser.

These pages are written in Markdown and rendered by Pluma itself: the same
`std/markdown` module parses this file into an AST, and `std/view` renders that
AST to the HTML you're reading. The docs are part of the app.

## Where to go next

- **[Language tour](/docs/tour/basics)** — learn the language from the ground up,
  one idea at a time. The best front-to-back introduction.
- **[Reference](/docs/reference)** — look things up: operators, types, error
  codes, and the [standard library](/docs/stdlib).
- **[Get started](/docs/start)** — install the compiler and run your first program.
- **[Guides](/docs/guides/cli)** — small, complete programs in each flavor Pluma is
  built for: a command-line script, a web server, a fullstack app.
- **[Playground](/playground)** — write and run Pluma in the browser.

## A taste

```pluma
use std/list

# Calls are uncurried and paren-free; parens only group sub-expressions.
def main = fun {
	let names = ["Ada", "Grace", "Edsger"]
	list.each names fun name {
		print "hello, $(name)!"
	}
}
```

Types are inferred, so you rarely write annotations — but you can, with `::`,
and the compiler checks them. Failures come back as values you handle, not
surprises that escape.

::: aside .callout
**New to Pluma?** Start with [Get started](/docs/start) to get the compiler on your
machine, then come back here for the details.
:::
