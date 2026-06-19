# How the fullstack build works

One command, `pluma build`, turns a project into the files a browser and a
server actually run. This page explains, in plain terms, what those files are and
how a page gets onto the screen, whether it's drawn on the server, in the
browser, or both. (This very site is built this way.)

## What a build produces

A fullstack project builds into two halves from one set of source files, a
server and a browser bundle:

```
main.wasm       the server: HTTP routes, your remote defs, page rendering
_built/         the client bundle the server serves for hydration
  loader.js     (loader.js + app.wasm, under the reserved /_built/ path)
  app.wasm
public/         your static assets (logo, fonts, ...), served as-is
```

Only the code the browser can actually reach is compiled into the client
`app.wasm`; server-only work like database access stays in `main.wasm`. You write
one program; the build splits it. The server *is* the deployable — there's no
standalone HTML/JS shell to host separately; everything is served by `main.wasm`.

`pluma run main.wasm` is self-sufficient: `app.serve` renders each page, routes
the `/_rpc/*` calls, and serves the `/_built/*` client bundle the page hydrates
with — so a production deploy is just the one server process, with no separate
static host required.

## Two ways a page reaches the screen

A page is a plain function that returns a `view` (a description of the UI as
data). That one description is consumed two ways, which gives the two paths below.

## Server-rendered (SSR): the default for this site

On each request the server runs your page function and turns the `view` into
finished HTML with `view.to-string`. The browser receives a fully-formed page and
paints it immediately, with no waiting for code to download and run first. Then
the browser bundle *hydrates* it: `render.hydrate` walks the same view alongside
the HTML the server already produced and re-attaches the interactive parts (event
handlers, anything reactive) to the existing elements, instead of rebuilding them.

```pluma
# server: render the view to HTML for this route
task.return (http.html 200 (theme.document-html "Title" (page ())))

# browser: adopt that server HTML rather than rebuild it
render.hydrate (dom.body ()) page
```

The payoff is a fast, styled first paint that works even before (or without)
JavaScript, plus full interactivity once the bundle loads.

## Client-rendered (CSR): a pure browser app

A project that's just a `client.pa` (no `main.pa`) builds a **static site**: there
is no server rendering the page. The browser loads a near-empty `index.html` shell,
and your code builds the whole DOM itself at startup with `render.mount`:

```pluma
# browser: build the DOM from scratch under <body>
render.mount (dom.body ()) page
```

Nothing renders until the bundle downloads and runs, but the output is just
static files: host it anywhere, no server required.

## Where the HTML comes from

Both paths start from the same `view`. The difference is only which consumer runs
it: `view.to-string` on the server produces an HTML string sent over the wire
(SSR), while `render.mount` in the browser builds real DOM nodes (CSR). Because
it's one description either way, the server and the browser can't disagree about
what the page should be, which is exactly what makes hydration safe.

## Where the CSS comes from

You style elements with `view.styled <rule>`, where a rule is an ordinary value.
Each rule is turned into a class whose name is a hash of its contents, and
registered in a collector as it's used. Identical rules collapse to one class
automatically, and the stylesheet only ever contains rules a page actually used.

Delivering that collected CSS mirrors the two paths:

On the **server (SSR)**, the page body is rendered first (which registers every
rule it uses) and then `css.style-tag ()` drops exactly those rules into an
inline `<style>` in the page's `<head>`. The page arrives already styled, with no
extra request.

In a pure **browser app (CSR)**, there's no server to emit the `<style>`, so the
runtime injects one from the collector when the app mounts, and refreshes it
whenever newly-shown content brings in a rule that wasn't on the page yet.

::: aside .callout
The through-line: there is one `view` and one set of style rules. SSR, CSR, and
the build-time `app.css` are just different times and places that same description
is turned into HTML and CSS, never a second, separate copy to keep in sync.
:::
