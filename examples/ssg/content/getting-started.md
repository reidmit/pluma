---
title: Getting started with Pluma
author: the pluma team
---

# Getting started

Install the toolchain, then write your first program.

## Hello, world

Every program has a `main`:

```
def main = fun {
	print "hello, world"
}
```

Run it with:

```
pluma run hello.pa
```

## A taste of types

Functions are annotated with `::`, and the compiler infers the rest:

```
def double :: fun int -> int = fun n { n * 2 }
```

You rarely *need* annotations — they're for documentation and for
pinning down ambiguous code. The type checker catches mistakes like
calling `double` on a `string` before your program ever runs.

## Next steps

1. Skim the [language tour](tour.html).
2. Read the standard library docs.
3. Build something — a parser, a CLI, even a *site generator*.

Questions? The [community](https://example.com/community) is friendly.
