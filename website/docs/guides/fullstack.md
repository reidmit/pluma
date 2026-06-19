# Fullstack app

A fullstack app is a directory with two entrypoints: `server.pa` and
`client.pa`. That pair is all `pluma build` needs to know it's building both
halves at once; mark a function `remote def` and the compiler writes both the
server route and the browser stub, so the two never drift apart.

Everything else is ordinary modules the two entrypoints `use`. This guide factors
the code they share into two more files (a convention, not a requirement) for
four small files in all:

```
app/
	api.pa      # shared:   the remote defs both sides agree on
	server.pa   # required: serves the page, dispatches RPC
	client.pa   # required: hydrates the server's HTML
	ui.pa       # shared:   the view, rendered on the server and hydrated in the browser
```

## api.pa

The contract. A `remote def` runs on the server; the browser calls it like a
local function.

```pluma
# api.pa -- the shared contract, compiled into both sides.
use std/task

# Runs on the server; the browser calls it as if it were local.
public remote def add :: fun int int -> task int = fun a b {
	task.return (a + b)
}
```

## server.pa

Serve the page for the first paint with `app.serve`, the fullstack server. It owns
two reserved route families so your handler stays focused on your own pages: the
`/_rpc/*` router the compiler generates from your remote defs, and the `/_built/*`
client bundle the browser hydrates with. Everything else falls through to `handler`.

```pluma
# server.pa -- serve the page; app.serve routes RPC calls and the client bundle.
use std/sys/app
use std/task
use std/sys/http
use ui

def handler :: fun http.request -> task http.response = fun req {
	if req.path == "/" {
		task.return (http.html 200 (ui.document ()))
	} else {
		task.return (http.not-found ())
	}
}

def main = fun {
	app.serve "127.0.0.1:8080" handler
}
```

## client.pa

Adopt the server's HTML instead of rebuilding it: no flash, no duplicate tree.

```pluma
# client.pa -- boot the browser by hydrating the server's HTML.
use std/web/dom
use std/web/render
use ui

def main = fun {
	render.hydrate (dom.body ()) ui.page
}
```

## ui.pa

The shared `view`, compiled into both halves. `page` is a pure view builder with no
effects at construction, so it renders the same on the server (`view.to-string`,
inside `document`) and the client (`render.hydrate`). The one effect, the remote
call, lives in a click handler, which runs only in the browser after hydration.

```pluma
# ui.pa -- the shared view: server-rendered, then hydrated in the browser.
use std/signal
use std/task
use std/view
use api

# A pure view: built once, rendered to HTML on the server and brought to life in
# the browser. The remote call sits in the click handler, which runs only after
# hydration; view.to-string drops it on the server.
public def page :: fun nothing -> view = using view {
	fun {
		let sum = signal.new 0
		.div [.id "app"] [
			.button [.on-click fun {
				task.spawn (task.map (api.add 2 3) fun n { signal.set sum n })
			}] [.text "add on the server"],
			.p [] [.text-of fun { "sum = $(to-string (signal.get sum))" }],
		]
	}
}

# The server wraps that view in a full HTML document for the first paint, and links
# the client bundle that hydrates it. Only the server calls this.
public def document :: fun nothing -> string = fun {
	"<!doctype html><html><body>$(view.to-string (page ()))<script type=\"module\" src=\"/_built/loader.js\"></script></body></html>"
}
```

So the two names from the files above are one pair: the server serves
`ui.document ()` (the whole HTML page) and the client hydrates `ui.page`, the
`view` embedded inside it.

Develop with live-reload, then build the deployable bundle:

```
pluma dev app/      # live-reload dev server
pluma build app/    # out/server.wasm + the client bundle in out/_built/
```

`pluma run out/server.wasm` is then self-sufficient: it renders the page, routes
the `/_rpc/*` calls, and serves the `/_built/*` client bundle the browser hydrates
with — no separate static file server in front of it.

::: aside .callout
Ready to build one? [Get started](/docs/start) gets the compiler on your machine, or
[browse the examples](https://github.com/reidmit/pluma/tree/main/examples) for
complete apps.
:::
