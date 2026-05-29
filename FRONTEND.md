# FRONTEND.md — the client MVU framework

**Status:** design, not started. Nothing here is built. The framework runs on the
**client/WASM target** (`IR.md` step 2 — the WASM emitter; its CPS state-machine and
`Repr` prerequisites have now landed, the emitter itself is still pending). The wire/RPC
protocol the frontend speaks to the server is `FULLSTACK.md`; the effect runtime it leans
on is `ASYNC.md`.

## Goal

A Model-View-Update (Elm-style) frontend, written in Pluma, sharing types and remote
calls with the server. The user writes three pieces — `init`, `update`, `view` — and the
runtime owns the model, the real DOM, and effect execution. Pure `update`/`view`,
type-safe messages, and remote calls that look like local async.

## Keystone: a command *is* a task

In Elm, `Cmd msg` is a bespoke opaque effect the runtime interprets. Pluma doesn't need
one: it already has `task a` (cold, scheduler-run, cancellable) and the whole
`task.map`/`attempt`/`??` combinator library (`ASYNC.md`). So:

> **`command msg` = `list (task msg)`** — a batch of tasks, each producing a `msg` the
> runtime feeds back into `update`. The empty list is "no effect" (Elm's `Cmd.none`); the
> list *is* `Cmd.batch`; the tasks run **concurrently on the structured-concurrency
> scheduler that already exists**.

There is no command API to invent — the task library *is* the command API, and an RPC
result flows into `update` as a `msg` for free. (It also forces the two-error-channel
model to surface honestly — see "Errors at the boundary".)

## The `app` type and the triad

`app model msg` is the value `client.pa`'s `main` produces. The three pieces:

```
init   :: (model, list (task msg))                    # initial model + initial commands
update :: fun msg model -> (model, list (task msg))   # pure step
view   :: fun model -> html msg                       # pure render
```

Two constructors, à la Elm's `sandbox`/`element`: **`app.sandbox`** for pure UIs with no
effects (`update :: fun msg model -> model`), and **`app.element`** for the full triad
above. The client entry just hands one over:

```
def main :: app model msg = app.element {init: init, update: update, view: view}
```

## The view layer

`view` returns an `html msg` — a virtual-DOM tree parameterized by the message its
handlers emit:

```
enum html msg {
	element string (list (attribute msg)) (list (html msg))   # tag, attrs, children
	text string
}
enum attribute msg {
	attr string string                  # static: class, value, href, …
	on   string (fun event -> msg)      # event handler: name → msg producer
}
```

with builders on top — `html.div`/`button`/`input`/`ul`/`li`/`text`, `attr.class`/`value`,
`events.on-click`/`on-input`/`on-submit`. The runtime diffs successive `view` outputs and
patches the real DOM through `core.dom` host imports (nodes held as `externref`); **the
diff is pure Pluma**. `html.map :: fun (fun a -> b) (html a) -> html b` re-tags a child
component's messages into a parent's `msg` — a plain generic function, no HKT.

## The runtime loop

1. Hold the current model; render `view model` → vdom; diff against the previous tree;
   patch the real DOM via `core.dom`, wiring handlers to dispatch `msg`.
2. On a `msg`: `(model', cmds) = update msg model`; commit `model'`.
3. Spawn each `cmd` (a `task msg`) on the root scope; each produced `msg` re-enters step 2.
4. Re-render. Concurrency, cancellation, and `defer` cleanup are the scope runtime's job
   (`ASYNC.md`) — already built.

## Errors at the boundary: the two-channel flattener

A command must turn the task's *failure* into a `msg` (the runtime has nowhere to put a
failing command), so you `task.attempt` it — which reifies the **transport** failure on
top of an endpoint's own **domain** `result` (`FULLSTACK.md`). That nests:
`result (result a domain-err) transport`. Unwrapping that in every `update` arm is
miserable, so the framework provides a flattener that folds both channels into one
`result a string` (or a richer error union):

```
# framework-provided; shown inline in the example below
call :: fun (task (result a e)) -> task (result a string)
```

This isn't optional sugar — it's load-bearing for usable command code. Its exact shape
(string vs. a structured error union; whether it stays generic over the domain error) is
an open question below.

## Worked example: a todo app

Three files: `todos.pa` (shared), `server.pa` (server entry), `client.pa` (the MVU app).
Uses the proposed, not-yet-built API (`server def`, `wire` auto-derive, `core.http`/
`core.db`, `html`/`events`/`app`). The shared and server mechanics are designed in
`FULLSTACK.md`; this example exists to show the *frontend* closing the loop end-to-end.

### `todos.pa` — shared

```
# The `todo` record auto-derives `wire` (every field is int/string/bool).
# Each `public server def` body is a SERVER ISLAND — it touches core.db/core.session,
# but the client only sees the signature and gets a stub, so those server-only imports
# never reach the client closure (target-gating stays happy with one shared file).
use core.task
use core.db
use core.session

public alias todo {id :: int, title :: string, done :: bool}

public enum todo-error {
	not-found
	empty-title
}

public server def list-all :: fun request -> task (result (list todo) todo-error) = fun req {
	try _user = session.user req            # auth = a plain `try`; transport-fails if unauthenticated
	try rows  = db.all "todos"
	ok rows
}

public server def add :: fun request string -> task (result todo todo-error) = fun req title {
	try _user = session.user req
	if title == "" is true {
		err empty-title                      # DOMAIN error → a wire'd `result` value
	} else {
		try saved = db.insert "todos" {title: title, done: false}
		ok saved
	}
}

public server def toggle :: fun request int -> task (result todo todo-error) = fun req id {
	try _user = session.user req
	try found = db.find "todos" id
	when found is some t {
		try saved = db.update "todos" {...t, done: t.done == false}
		ok saved
	} is none {
		err not-found
	}
}

public server def remove :: fun request int -> task (result (list todo) todo-error) = fun req id {
	try _user = session.user req
	try _     = db.delete "todos" id
	try rows  = db.all "todos"
	ok rows
}
```

### `server.pa` — server entry

```
# Mounts the generated RPC dispatch (one route per `public server def`) and serves the
# wasm bundle `pluma build` produced. core.http is illustrative.
use core.http

def main = fun {
	let routes = http.with-client (http.with-rpc (http.router ()))
	print "serving on http://localhost:8080"
	http.listen routes 8080      # task nothing — runs until killed
}
```

### `client.pa` — the MVU app

```
use core.task
use core.list
use todos                        # shared module: todos.todo, todos.list-all, …

alias model {
	items :: list todos.todo,
	draft :: string,
	error :: option string
}

enum msg {
	loaded (result (list todos.todo) string)
	draft-changed string
	submit
	added (result todos.todo string)
	toggle int
	toggled (result todos.todo string)
	remove int
	removed (result (list todos.todo) string)
}

# The two-channel flattener (belongs in the framework; shown inline). Collapses an
# endpoint call into one `task (result a string)`, folding TRANSPORT failure and DOMAIN
# error into a single human message.
def call :: fun (task (result a todos.todo-error)) -> task (result a string) = fun t {
	task.map (task.attempt t) (fun outcome {
		when outcome is ok inner {
			when inner is ok v { ok v } is err de { err (describe de) }
		} is err _transport {
			err "network error"
		}
	})
}

def describe :: fun todos.todo-error -> string = fun e {
	when e is not-found { "todo not found" } is empty-title { "title can't be empty" }
}

def init :: (model, list (task msg)) =
	({items: [], draft: "", error: none},
	 [task.map (call (todos.list-all (request.new ()))) (fun r { loaded r })])

def update :: fun msg model -> (model, list (task msg)) = fun m model {
	when m is loaded r {
		when r is ok xs { ({...model, items: xs, error: none}, []) }
		is err e { ({...model, error: some e}, []) }
	}
	is draft-changed s { ({...model, draft: s}, []) }
	is submit {
		if model.draft == "" is true {
			(model, [])
		} else {
			let cmd = task.map (call (todos.add (request.new ()) model.draft)) (fun r { added r })
			({...model, draft: ""}, [cmd])
		}
	}
	is added r {
		when r is ok t { ({...model, items: [...model.items, t], error: none}, []) }
		is err e { ({...model, error: some e}, []) }
	}
	is toggle id {
		(model, [task.map (call (todos.toggle (request.new ()) id)) (fun r { toggled r })])
	}
	is toggled r {
		when r is ok t {
			({...model, items: list.map model.items (fun x { if x.id == t.id is true { t } else { x } }), error: none}, [])
		} is err e { ({...model, error: some e}, []) }
	}
	is remove id {
		(model, [task.map (call (todos.remove (request.new ()) id)) (fun r { removed r })])
	}
	is removed r {
		when r is ok xs { ({...model, items: xs, error: none}, []) }
		is err e { ({...model, error: some e}, []) }
	}
}

def view :: fun model -> html msg = fun model {
	html.div [attr.class "todo-app"] [
		view-error model.error,
		html.form [events.on-submit submit] [
			html.input [attr.value model.draft, attr.placeholder "what needs doing?",
			            events.on-input (fun s { draft-changed s })] [],
			html.button [] [html.text "add"]
		],
		html.ul [] (list.map model.items view-item)
	]
}

def view-item :: fun todos.todo -> html msg = fun t {
	html.li [] [
		html.input [attr.type "checkbox", attr.checked t.done, events.on-click (toggle t.id)] [],
		html.span [] [html.text t.title],
		html.button [events.on-click (remove t.id)] [html.text "x"]
	]
}

def view-error :: fun (option string) -> html msg = fun e {
	when e is some m { html.div [attr.class "error"] [html.text m] } is none { html.text "" }
}

def main :: app model msg = app.element {init: init, update: update, view: view}
```

### What it exercises

- **`wire` auto-derive** — `todo` crosses the wire with zero annotation.
- **Server islands** — `todos.pa` is shared yet uses `core.db`/`core.session`; the client
  closure stops at the `server def` bodies, so target-gating passes without splitting the file.
- **Commands = tasks** — every effect in `update` is a `task msg` built with `task.map`;
  the scheduler runs them and feeds results back as messages.
- **Two error channels** — transport rides the task failure, domain rides the `result`
  return; the `call` flattener folds them for the UI.
- **Record update** — `{...model, items: xs, error: none}` and `{...t, done: …}` keep
  `update` and the server's `toggle` readable (the Elm-style update-only, type-preserving
  form now in the language).

## Deferred / open questions

- **Subscriptions** (external streams: ticks, websockets, keyboard). Deferred from core;
  recurring needs can use self-re-issuing commands for now. The real version has a natural
  home as `subscriptions :: fun model -> list (subscription msg)` that the runtime diffs
  against model changes, each backed by a scope-spawned fiber that's `s.cancel`'d when it
  drops — reusing the structured-concurrency runtime.
- **The `call` flattener's final shape** — string message vs. a structured error union;
  whether it stays generic over the domain error type. It's framework surface, not optional.
- **`init` flags / SSR hydration** — the seam where a `wire`'d, server-rendered initial
  model would arrive, so the first paint isn't empty.
- **Event model** — typed handlers (`on-click`, `on-input`, `on-submit`) plus a generic
  `on name (fun event -> msg)` over an opaque `event` with accessors; confirm the shape.
- **Server-driven per-route mode** — client-MVU is the default (responsive, network-frugal,
  offline-capable). A low-interactivity / data-heavy route *could* opt into server-side
  render+diff (LiveView-style) to shed client weight, at the cost of a round-trip per
  interaction and per-connection server state. Worth offering per-route later, not as the
  default — the diff is cheap, the round-trip is what's expensive.

## Relationship to other docs

- `FULLSTACK.md` — the wire codec, RPC mechanism, build/entry, and target-gating this
  frontend speaks to.
- `ASYNC.md` — `task`/`scope`; the command runtime *is* the structured-concurrency scheduler.
- `IR.md` — the WASM backend (the frontend's compile target).
