# Fullstack app

A fullstack app is a directory with a `server.pa` and a `client.pa`. Mark a
function `remote def` and the compiler writes both the server route and the
browser stub, so the two never drift apart. Four small files:

```
app/
	api.pa      # shared contract: the remote defs
	server.pa   # serves the page, dispatches RPC
	client.pa   # hydrates the server's HTML
	ui.pa       # the shared view (server + browser)
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

Serve the page for the first paint, and hand everything else to the router the
compiler generates from your remote defs.

```pluma
# server.pa -- serve the page, let the compiler route the RPC calls.
use std/sys/rpc
use std/task
use std/sys/http
use ui

def handler :: fun http.request -> task http.response = fun req {
	if req.path == "/" {
		task.return (http.html 200 (ui.document ()))
	} else {
		rpc.dispatch req
	}
}

def main = fun {
	http.serve "127.0.0.1:8080" handler
}
```

## client.pa

Adopt the server's HTML instead of rebuilding it — no flash, no duplicate tree.

```pluma
# client.pa -- boot the browser by hydrating the server's HTML.
use std/web/dom
use std/web/render
use ui

def main = fun {
	render.hydrate (dom.body ()) ui.page
}
```

The `ui.pa` module holds the shared `view` — built once, rendered to HTML on the
server and brought to life in the browser. Develop with live-reload, then build
the deployable bundle:

```
pluma dev app/      # live-reload dev server
pluma build app/    # server .wasm + browser bundle
```

::: aside .callout
Ready to build one? [Get started](/start) gets the compiler on your machine, or
[browse the examples](https://github.com/reidmit/pluma/tree/main/examples) for
complete apps.
:::
