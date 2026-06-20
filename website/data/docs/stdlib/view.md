# Views and HTML

`std/view` is how you describe a user interface in Pluma. The central idea: a
**view is pure data**, a tree of elements, built with ordinary function calls. It
never touches the screen itself. Two renderers turn it into output: one builds
real DOM in the browser, and `view.to-string` renders it to HTML on the server.
The same view works in both places, which is what lets a page arrive
fully-formed and then come alive in the browser.

## Building elements

There's one builder per HTML tag (`view.div`, `view.h1`, `view.p`,
`view.button`, `view.a`, and so on), and each takes two lists: the element's
*attributes* and its *children*. `view.text` makes a text node.

```pluma
use std/view

view.div [view.class "card"] [
	view.h1 [] [view.text "Hello"],
	view.p [] [view.text "Welcome to Pluma."],
]
```

Writing `view.` on every call gets noisy fast, so view code is the natural home
for a [`using` block](/docs/reference/using), which lets you drop the prefix and
write the leading-dot form:

```pluma
use std/view

using view {
	.div [.class "card"] [
		.h1 [] [.text "Hello"],
		.p [] [.text "Welcome to Pluma."],
	]
}
```

## Components are just functions

A component is nothing special: it's a function that returns a `view`. You
compose interfaces by nesting builders and by calling other components, the same
way you call any function:

```pluma
use std/view

def card :: fun string string -> view = using view {
	fun title body {
		.div [.class "card"] [
			.h2 [] [.text title],
			.p [] [.text body],
		]
	}
}
```

Rendered on the server, `view.to-string (card "Hello" "Welcome to Pluma.")`
produces:

```
<div class="card"><h2>Hello</h2><p>Welcome to Pluma.</p></div>
```

## Attributes, styles, and events

Attributes are values too, built with their own helpers. `view.class` and
`view.href` set the common HTML attributes, `view.attr` sets an arbitrary one,
and `view.styled` attaches a scoped style rule built with
[`std/css`](/std/css), the styling approach the rest of the site uses. Event
handlers are attributes as well: `view.on-click`, `view.on-input`, `view.on-submit`.

```pluma
use std/view

view.a [view.href "/docs", view.class "link"] [view.text "Read the docs"]
view.button [view.on-click (fun { save () })] [view.text "Save"]
```

A handler runs in the browser when the event fires; on the server it's simply not
called, since `to-string` only renders markup.

## State and reactivity

For a view that *changes*, hold the changing data in a **signal** (a cell you
read and write), and anything that reads it updates on its own when it changes.
You make local state with `signal.new` right inside a component, and bind the
reactive parts with helpers like `view.text-of` (text that recomputes):

```pluma
use std/view
use std/signal

def counter :: fun int -> view = using view {
	fun start {
		let n = signal.new start
		.div [] [
			.button [.on-click (fun { signal.update n (fun x { x - 1 }) })] [.text "-"],
			.text-of (fun { to-string (signal.get n) }),
			.button [.on-click (fun { signal.update n (fun x { x + 1 }) })] [.text "+"],
		]
	}
}
```

Drop `counter 0` and `counter 100` side by side and each keeps its own state, with
no extra wiring. Beyond reactive text, `view.show` renders a child only when a
condition holds, `view.dyn` swaps a whole subtree as data changes, and `view.each`
renders a list, re-rendering only the rows that actually changed. The
[signals deep-dive](/docs/deep-dives/signals) explains how the tracking works
under the hood; you don't need those internals to build a UI.

## Rendering

The same view reaches the screen two ways. On the server, `view.to-string` turns
it into HTML. That's server-side rendering, and it's also how headless tests
check a view without a browser. In the browser, the `std/web/render` module
mounts the view as live DOM and wires up the signals and handlers. You write the
view once; the renderer is chosen by where it runs.

## See also

- **[Reactive frontend with signals](/docs/deep-dives/signals)**: how
  fine-grained reactivity tracks dependencies and updates the DOM.
- **[Server-side rendering](/docs/deep-dives/ssr)**: rendering on the server and
  hydrating in the browser.
- **[Using blocks](/docs/reference/using)**: the ambient-`view` syntax these
  examples lean on.
