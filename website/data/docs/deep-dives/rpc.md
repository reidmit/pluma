# How RPC works

Most apps just call a `remote def` and get back a value, and the network is
invisible. This page is for the curious: how a `remote def` becomes a real HTTP
call, what the compiler generates, and where the boundary between client and
server actually sits.

::: aside .callout
**You don't need this to build apps.** The [Fullstack app](/docs/guides/fullstack)
guide shows the happy path. Read on if you want to understand what the compiler is
doing underneath.
:::

## The `remote def` contract

A `remote def` is the seam between the two halves of a fullstack app. Its
signature is a plain logical contract (the arguments and the result) with no
mention of the network:

```pluma
use std/task

public remote def greet :: fun string -> task string = fun name {
	task.ok ("hello, " ++ name)
}

public remote def add :: fun int int -> task int = fun a b {
	task.ok (a + b)
}
```

The body you write is the *server's* handler. The signature, though, belongs to
both sides, and that's the whole trick. The browser calls it like any local
async function:

```pluma
try sum = api.add 2 3   # => 5, fetched from the server
```

Because there is exactly one signature, and the compiler type-checks both the
call site and the handler against it, the two ends can't disagree about the shape
of the data. End-to-end type safety isn't enforced by discipline; it falls out of
checking one definition twice. Notice too that there's no `request` parameter: the
contract is the *logical* call, not the HTTP mechanics underneath.

## What the compiler generates

A fullstack project compiles into two programs from one source tree: a server
and a browser bundle (see [the build](/docs/reference/build)). A `remote def` is
where the two are stitched together, and the compiler gives it a different body in
each:

- On the **server**, it keeps the handler you wrote, and adds the endpoint to a
  generated dispatcher that routes an incoming `/_rpc/<name>` request to it.
- On the **client**, it replaces the body with a generated transport stub: encode
  the arguments, POST them, decode the reply.

You don't wire the dispatcher in yourself: `app.serve` owns the reserved `/_rpc/*`
routes (alongside the `/_built/*` client bundle), and hands everything else to the
handler you write — so your handler only ever sees your own pages:

```pluma
def main = fun {
	app.serve "127.0.0.1:8080" handler   # owns /_rpc/* and /_built/*
}

def handler :: fun http.request -> task http.response = fun req {
	if req.path == "/" {
		task.ok (http.html 200 (ui.document ()))
	} else {
		task.ok (http.not-found ())
	}
}
```

The split also decides what ships to the browser: only code the client can
actually reach is compiled into the bundle. A `remote def`'s *body*, the
server-side handler and whatever it calls, like [database
access](/docs/stdlib/database), stays on the server. The browser gets the stub,
not the implementation.

## The wire and the transports

A call's arguments are serialized with Pluma's binary `wire` codec (compact, and
typed, so there's no JSON-shaped guessing about what a field holds), POSTed to
`/_rpc/<name>`, and the reply is decoded the same way. Two transports implement
that round-trip with the same shape: the browser goes through the host page's
`fetch`, and a native client goes through [`http.fetch`](/docs/stdlib/http). The
generated stub calls whichever one the build target installed.

One danger with any client/server split is *drift*: you redeploy the server with a
changed endpoint, but an old browser tab is still running the previous client. If
the bytes happened to line up, that stale client could silently misread the new
reply. Pluma closes that hole with a **schema fingerprint**: each route carries a
hash of its argument and result types. When a call's fingerprint doesn't match the
server's, the server answers `409` and the client surfaces a typed
"skew" error instead of decoding garbage. A stale client fails loudly and
honestly.

## Two kinds of failure

A remote call can fail two very different ways, and Pluma keeps them on separate
channels:

- A **domain** failure ("no such user", "insufficient funds") is part of what
  the endpoint means, so it lives in the result: the endpoint returns `task
  (result a e)`, and the caller handles the `result` as usual.
- An **infrastructure** failure (the network is down, the request was
  unauthorized, the schema drifted) isn't any one endpoint's business. These ride
  the task's failure channel as an `rpc-error`, which you recover with
  `task.attempt` when you want to inspect it:

```pluma
try outcome = task.attempt (api.add 2 3)
when outcome is ok sum {
	# the call went through
} is err e {
	# e is an rpc-error: transport, unauthorized, skew, ...
}
```

Keeping auth failures (401/403) here, rather than in every endpoint's payload,
means a single `remote def` doesn't have to widen its result type just because a
request might be rejected.

## Ambient context and rejection

The contract has no `request` parameter, but a handler sometimes needs a fact
about the request anyway, like an auth header. Rather than thread a `request`
through every signature, the dispatcher binds the inbound request for the duration
of each handler call, and the handler reads what it needs from
`std/sys/rpc/context`:

```pluma
use std/sys/rpc/context

# inside a remote def's handler, on the server:
when context.header "authorization" is some token {
	# validate the token
} is none {
	rpc.reject 401   # fail this call with an HTTP status
}
```

`rpc.reject` ends the handler with a status code, which the client receives as the
matching `rpc-error` (an `unauthorized` for 401/403). On the outbound side,
`rpc.with-headers` scopes extra headers onto the calls made within it, for
attaching that same auth token to a request from one service to another. Identity
itself is left to your app: the context carries transport facts, and you build
authentication on top.

## See also

- **[Fullstack app](/docs/guides/fullstack)**: the `remote def` happy path, end
  to end.
- **[Fullstack build](/docs/reference/build)**: how one source tree becomes a
  server and a browser bundle.
- **[Concurrency](/docs/reference/concurrency)**: the `task` a remote call
  returns and `task.attempt` for recovering its failure.
