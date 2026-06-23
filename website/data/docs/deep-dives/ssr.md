# Server-side rendering

When a fullstack app serves a page, the server renders the view to HTML first, so
the browser shows real content before any code runs. The browser then *hydrates*
that markup, taking over the existing DOM and wiring up reactivity without
rebuilding it. This page explains the round trip.

::: aside .callout
**You don't need this to build apps.** Server rendering is on by default. Read on
if you want to understand the handoff between server and browser.
:::

If you just want to know what files a build produces and which rendering path a
page takes, the [fullstack build](/docs/reference/build) reference covers that.
This page goes a level deeper, into how the server's HTML and the browser's
reactivity actually meet.

## One view, rendered to HTML

A page is a function returning a [`view`](/docs/stdlib/view), a description of
the UI as data. On the server, `view.to-string` walks that tree and produces an
HTML string, which the server sends as the page body:

```pluma
# server: wrap the page's view in the full HTML document for this route
task.ok (http.html 200 (app.document { lang: "en", title: "Title", head: [], body: page () }))
```

The browser receives finished, styled markup and paints it immediately, with no
waiting for a bundle to download and build the DOM first. That's the whole point
of rendering on the server: a fast first paint that works even before the
JavaScript loads.

Crucially, it's the *same* `view` the browser will use. The server doesn't
produce one description of the page and the client another: there's a single
function, run two ways, so the two can't drift out of agreement. That agreement is
what makes the next step safe.

## Carrying the data across

A page is rendered from some data: the rows for this user, the post being viewed.
The server has that data in hand when it renders. For the browser to hydrate the
*same* tree, it needs the *same* data, and the obvious approach (have the client
re-fetch it over the network) is wasteful: it's a second round-trip, and it opens
a window where the page is visible but not yet interactive.

So the server embeds the data in the page instead. It serializes the data once and
writes it into a non-executed `<script type="application/pluma">` tag, and the
browser reads it straight back with `render.boot-data` before it builds the view:

```pluma
# browser: read the data the server embedded, then hydrate from it
when render.boot-data "page-data" is ok data {
	render.hydrate (dom.body ()) (fun { page data })
} else {
	# no embedded data (served as a plain client shell): fetch it instead
	render.mount (dom.body ()) (fun { loading () })
}
```

Because `boot-data` returns a [`result`](/docs/reference/errors), the same app
works whether or not it was server-rendered: when the embedded data is missing, it
falls back to fetching. No separate client-only and server-only versions to keep
in step.

## Hydration: adopting, not rebuilding

`hydrate` is the third way to consume a view, alongside `view.to-string` (server)
and `render.mount` (build DOM from scratch). Instead of creating nodes, it walks
the view tree alongside the DOM the server already produced and **adopts** each
existing node, attaching effects to the reactive parts and listeners to the
handlers, while leaving the static structure untouched.

That's why hydration causes no flash and no re-render: the nodes are already on
screen; the browser is only threading reactivity through them. The view tree and
the server's DOM line up one-to-one in document order, so children match by
position.

That one-to-one alignment is also the thing to keep in mind when it goes wrong. If
two reactive text nodes sit directly next to each other, the browser collapses
them into a single text node when it parses the server's HTML, and now the view
expects two children where the DOM has one, knocking the alignment off. The fix is
to give reactive text its own element so it stays a distinct node:

```pluma
# instead of a bare reactive text node, wrap it:
view.span [] [view.text-of (fun { to-string (signal.get n) })]
```

## The two halves of the build

All of this assumes one program compiled into both a server and a browser bundle.
The build figures out which code each side needs and splits accordingly:
server-only work like [database access](/docs/stdlib/database) never reaches the
browser. The [fullstack build](/docs/reference/build) reference covers what's
produced, and [how RPC works](/docs/deep-dives/rpc) covers how a `remote def`
bridges the two halves with a typed call.

## See also

- **[Fullstack build](/docs/reference/build)**: the files a build produces and
  the SSR-versus-CSR choice.
- **[Reactive frontend with signals](/docs/deep-dives/signals)**: the reactivity
  that hydration re-attaches to the server's DOM.
- **[Views and HTML](/docs/stdlib/view)**: `view.to-string` and the element
  builders the server renders.
