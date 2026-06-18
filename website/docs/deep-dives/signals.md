# Reactive frontend with signals

Pluma's frontend is built on fine-grained reactivity: a *signal* holds a value,
and anything that reads it re-runs when it changes. There's no virtual DOM and no
update loop: the framework tracks exactly which views depend on which data and
updates only those. This page explains how that works.

::: aside .callout
**You don't need this to build apps.** The view layer reads naturally without
knowing the internals. Read on if you want to understand why it updates the way it
does.
:::

## Three pieces

`std/signal` has three building blocks:

- A **signal** is a cell you can read and write: `signal.new` creates one,
  `signal.get` reads it, `signal.set` writes it.
- A **computed** is a derived value: `signal.computed` takes a function, and the
  result acts like a read-only signal that recomputes when its inputs change.
- An **effect** is a side-effecting reader: `signal.effect` runs a function now,
  and re-runs it whenever any signal it read has changed.

```pluma
use std/signal

let n = signal.new 0
let doubled = signal.computed (fun { (signal.get n) * 2 })
let _ = signal.effect (fun {
	print "doubled is $(to-string (signal.get doubled))"
})
# prints "doubled is 0" immediately

signal.set n 5
# the effect re-runs on its own, printing "doubled is 10"
```

You never tell the effect what it depends on. It read `doubled`, which read `n`,
so it depends on both, and only those.

## Automatic dependency tracking

The magic is in `signal.get`. While an effect or computed is running, every
`signal.get` it makes quietly records that *this reader depends on that signal*.
Pluma keeps track of which reader is currently running and links it to each signal
it touches. When you later `set` that signal, it knows exactly which readers to
wake.

That's why dependencies are always exact and never go stale: they're discovered by
*running* your code, not declared in a list you have to keep in sync. Read a signal
inside an `if` branch that didn't run this time, and you simply don't depend on it
this time. There's nothing to register by hand and nothing to forget.

## Updates without glitches

A naive version of this idea has a famous bug. Suppose an effect reads two
computeds that both depend on the same signal, a diamond:

```
        n
       / \
   left   right
       \ /
      effect
```

Change `n`, and a naive system might update `left`, run the effect, then update
`right` and run the effect *again*. And in between, the effect saw a `left` and
`right` that disagreed about `n`. That inconsistent in-between is called a glitch.

Pluma avoids it with a *pull* model. A `set` doesn't eagerly recompute anything:
it marks the dependent computeds as stale and queues the dependent effects. The
computeds don't actually recompute until something reads them, so by the time the
effect runs and pulls `left` and `right`, both are already fresh, and the effect
runs **once**, seeing a consistent world:

```pluma
use std/signal

let a = signal.new 1
let left = signal.computed (fun { (signal.get a) + 1 })
let right = signal.computed (fun { (signal.get a) + 2 })
let runs = ref.new 0
let _ = signal.effect (fun {
	let _l = signal.get left
	let _r = signal.get right
	ref.update runs (fun x { x + 1 })   # count how often the effect runs
})

signal.set a 10
ref.get runs   # => 2  (once on creation, once for the update, not three times)
```

When you want several writes to settle as a single update, wrap them in
`signal.batch`: every effect still runs at most once, after all the writes land.

One more economy: `set` compares the new value to the current one (by value), and
if they're equal it does nothing at all: no store, no notify. A redundant write
is free, and an effect that happens to write back an unchanged value settles
instead of looping forever.

```pluma
let m = signal.new 7
let runs = ref.new 0
let _ = signal.effect (fun {
	let _v = signal.get m
	ref.update runs (fun x { x + 1 })
})
signal.set m 7      # same value, the effect does NOT re-run
ref.get runs        # => 1
```

## Cleaning up: the owner tree

A UI creates and destroys effects constantly, every time a list row appears or a
panel closes. If those effects kept running after their part of the page was gone,
they'd leak. So effects are arranged in an **owner tree**: a dynamic piece of UI
runs under its own owner, and disposing that owner disposes exactly the effects
created beneath it, no more and no less.

`signal.owned` runs a function under a fresh owner and hands back a dispose
function; `signal.on-cleanup` registers a teardown to run when the current owner
is disposed. The view layer uses these so that when a `view.show` hides its child
or a `view.each` row drops out, that subtree's effects are torn down with it
automatically; you don't manage any of it. (Each `signal.effect` also returns its
own dispose function for standalone use, which is the `_` we've been ignoring
above.)

## How the view layer rides on this

[`std/view`](/docs/stdlib/view) is a thin reactive layer over signals. The
reactive builders are just effects in disguise: `view.text-of` is text backed by
an effect that re-runs when its signal changes, `view.show` and `view.dyn` swap
content under an owner that's disposed when the condition flips, and `view.each`
keys a list so only the rows that actually changed are touched.

In the browser, `std/web/render` walks the view once and attaches each of these
effects directly to a real DOM node. After that first pass there's no diffing and
no retained shadow tree: a `set` flows straight through the effect to the one DOM
node that depends on it. That directness is the whole reason the model is fast:
the framework never re-examines parts of the page that didn't change, because it
already knows, edge by edge, what depends on what.

## SSR

Because signals are just a graph over ordinary values, the same reactive code runs
on the server too, which is what lets a page render to HTML and then come alive in
the browser. The [server-side rendering](/docs/deep-dives/ssr) deep-dive covers
that handoff.

## See also

- **[Views and HTML](/docs/stdlib/view)**: the reactive builders that wrap these
  primitives.
- **[Server-side rendering](/docs/deep-dives/ssr)**: rendering a signal-driven
  view on the server and hydrating it in the browser.
