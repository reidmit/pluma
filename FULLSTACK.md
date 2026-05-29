# FULLSTACK.md — end-to-end-typed client/server Pluma

**Status:** design, not started. Nothing here is built. The codec and the RPC
plumbing are **prototypable on the VM today** (no WASM dependency); the actual
browser client is a separate, larger milestone — the WASM/WasmGC backend and the
Elm-style frontend are tracked in `IR.md` (step 2) and are out of scope here.

## Goal

One language, fullstack. Pluma already targets scripts, CLIs, and (server-side)
long-running processes; this doc designs the layer that lets you also write the
*frontend* and have it talk to the *backend* with **end-to-end type safety** — no
hand-written serialization, no schema duplication, no untyped JSON glue.

The whole design rests on one spine:

> **A remote call is just a server-side async function, and `try` over it is
> identical to a local await.**

```
# shared/users.pa — compiled into BOTH client and server
public server def fetch :: fun request int -> task (result user not-found) = fun req id {
    try maybe = db.find-user id
    when maybe is some u { ok u } is none { err not-found }
}
```

On the client, calling `users.fetch` serializes the args, does the round-trip, and
hands back a `task` — so the call site is just `try u = users.fetch (request.new ()) id`,
the same syntax as any local await. The end-to-end safety isn't a separate checker:
client and server compile **one signature from one source** into two targets, so the
types can't drift. Everything below rides machinery Pluma already has — `task`/`try`/
`??` (async), `result` (errors), traits + dictionary passing (the codec), and module
visibility + target-gating (the client/server split).

There are two layers: the **wire codec** (how values become bytes) and the **RPC
mechanism** (how a call crosses the network). They're independent — the codec is
useful on its own (persistence, caching, queues).

---

# Layer 1 — the `wire` codec

## The trait

```
trait wire a {
    to-wire   :: fun a -> bytes
    from-wire :: fun bytes -> result a wire-error
}
```

- **Silent structural auto-derive.** Any type built from `int`/`float`/`bool`/
  `string`/`duration`/records/tuples/lists/enums is automatically `wire` — no
  `derive` annotation, "it just works if the shape works." This mirrors how `==` is
  already structural: you don't implement equality, you get it.
- **Still a real trait.** Auto-derivation is the default, not the only path: a type
  can supply a hand-written instance, exactly like the existing
  `implement ord (option a) where (ord a)` parametric instances. This is how opaque
  types and custom wire formats opt in (see below).
- **Implementation.** A built-in resolution rule in the constraint solver —
  `can_derive_wire(ty)`, recursive, bottoming out at primitives — plus codegen that
  synthesizes the dictionary. Modeled on the built-in `numeric`/`ord` instances, *not*
  materialized instance AST.
- **Public methods are `bytes`-granularity; the generated recursive internals thread
  a cursor** (`reader -> result (a, reader)`), since decoding nested compounds needs
  position tracking. Clean public surface, cursor-based internals.

## Why the two directions are asymmetric (and why it's a trait, not a builtin)

Serialize *could* be a single runtime builtin that recurses over a value, like `==`
— a value knows its own shape. **Deserialize cannot**: a byte buffer doesn't know it's
supposed to become an `option int` vs a record. The *target type* has to drive the
parse, and that type only exists statically. So deserialize is inherently
type-directed → it's a derived (per-type) trait, not a runtime function. Once
deserialize is type-directed, serialize follows for symmetry.

## Format: compact schema-driven binary, **not JSON**

The receiver always knows the exact static type — that's the premise of end-to-end
typing. So **the type *is* the schema**, and JSON's self-description (field names,
`{"some": …}` tags, quotes, decimal numbers) is dead weight. `wire` emits a compact
binary encoding straight to `bytes`, with no intermediate `json.value` tree:

| Pluma | wire bytes |
|---|---|
| `int` | varint, zigzag |
| `float` | raw 8-byte IEEE-754 |
| `bool` | 1 byte |
| `string`, `list a` | varint length prefix + contents |
| tuple, **record** | fields in declared order — **no field names on the wire** |
| enum | varint tag + payload |
| `duration` | its constant int repr |

This is essentially **borsh / protobuf-without-field-tags**: positional,
deterministic, and a tight linear encode/decode. Determinism is a bonus — identical
values produce identical bytes, so a payload is content-addressable (cache keys,
signatures, dedup), which JSON can't give without canonicalization. Binary leans
*harder* on the end-to-end type guarantee than JSON does — which is fine, because that
guarantee is the whole point.

`core.json` stays exactly as it is (the `json.value` ADT + `parse`/`stringify`
builtins), as a **separate, explicit** tool for the cases that genuinely want it:
public APIs, non-Pluma callers, webhooks, config, human inspection. `wire` is
deliberately **not** pluggable across encodings (no encoder-abstraction typeclass) —
two concrete tools, each good at one job. An endpoint opts into JSON when it needs to
talk to the outside world.

## Version skew: a schema fingerprint, not field tags

Positional binary is fragile to client/server drift — a stale client decoding a
new server's payload gets garbage, not a clean error. protobuf solves this with
per-field tags; we don't want to pay that compactness cost. Instead:

- The build computes a **schema fingerprint** — a hash over every `wire`-reachable
  type definition in the endpoint surface.
- It rides in a **per-request header**.
- On mismatch, the call fails with a clean `wire-error` ("client out of date,
  reload"), surfaced through the client's `try`/`??` — never a corrupt decode.

The **supported deploy model is atomic, compiled-together deploy**: client and server
are one build. Stale-client-against-new-server is *not* required to keep working
transparently; it's required to fail *cleanly*. That decision is what lets us keep the
format maximally compact.

## The boundary is just a trait constraint

A remote signature requires every argument and result type to be `wire`. That's an
ordinary constraint — `where (wire arg) (wire ret)` — solved by the existing
machinery. Two consequences fall out for free:

- **Non-data types are rejected at compile time.** `fun`, `ref a`, `task a`, `regex`
  aren't derivable, so they physically cannot appear in a remote signature. The error
  lands at the boundary, with attribution ("can't send field `on-click`: functions
  aren't serializable"), not at runtime.
- **Opaque enums are non-derivable by construction.** Their constructors are hidden, so
  the compiler can't synthesize `from-wire`. A module that wants its opaque type on the
  wire must export a **hand-written `wire` instance**, exactly mirroring how it already
  exports smart constructors. The visibility system does real work here — and it's the
  right default: opaque internals shouldn't leak to the wire by accident.

---

# Layer 2 — the RPC mechanism

## Marking an endpoint: `public server def`

`server` is a prefix modifier on `def`, composing with the visibility ladder but
sitting in its own slot — `public`/`opaque` say *who can see it*, `server` says *where
it runs*:

```
public server def fetch :: fun request int -> task (result user not-found) = fun req id {
    try maybe = db.find-user id          # db is server-only — legal, this body is a server island
    when maybe is some u { ok u } is none { err not-found }
}
```

- **The body is a server-target island.** It can freely use server-only modules
  (`core.db`, `core.fs`) even inside an otherwise-shared module, because the body is
  only ever compiled for the server. On the **client** target the compiler **discards
  the body** and emits a stub from the signature. The signature is the contract that
  crosses; the body never does — same principle as `wire` data.
- **`server` does not widen visibility.** An endpoint must be reachable by the client
  to be callable, so it's always written `public server def`; a private `server def` is
  a mistake worth flagging, not a silent widening (Pluma stays explicit and
  private-by-default).
- Endpoints are inherently def-level — the route derives from the qualified name (e.g.
  `users.fetch` → `/rpc/users.fetch`).

## Request context: a symmetric `request` param carrying credentials, not identity

Both client and server take the **same** `request` param of the **same** type — no
stripped/derived signature asymmetry. The client uses it to attach transport metadata
(custom headers, trace ids, idempotency keys); the server reads what arrived.

```
# client and server share the identical type `fun request string -> task ...`
public server def create-post :: fun request string -> task (result post forbidden) = fun req body {
    try author = session.user req        # server-only: reads req's auth header/cookie, VALIDATES → user
    db.insert-post author body
}

# client
try outcome = posts.create-post (request.new ()) "hello world"
try outcome = posts.create-post (request.header (request.new ()) "x-trace-id" trace) "hello world"
```

The critical move: **identity is never a field on `request`.** A client presents
*credentials* (an `authorization` header / cookie); the server *derives and validates*
the authenticated user via a server-only function (`session.user req`). There is no
`req.user` to forge. This is exactly how HTTP already works — headers are
client-controlled, identity is server-validated — so it's both familiar and secure by
construction.

**The asymmetry lives in *which functions are callable*, not in the type.** `request`
is one symmetric type; `session.user`, `net.peer-addr`, verified-session accessors,
etc. live in **server-only modules**, so on the client they simply don't exist —
no meaningless empty fields, no special RPC rule. That difference is *just
target-gating*, the same mechanism used everywhere else. Server-observed facts (peer
IP, TLS info) are server-only *queries over* the request, not fields on it.

We rejected the alternatives: an **ambient** `request.current ()` accessor (hidden
dynamic-scope effect — contradicts the async design's "no hidden effects, `task` is an
honest annotation" principle, and isn't testable without ambient setup), and a
**client-strips-the-param** scheme (the signature asymmetry that started this section).

Minor open ergonomic: `request.new ()` on every call is slightly noisy; a terser
builder or an implicit default for metadata-less calls can come later. The transport
auto-attaches cookies regardless of `request`, so the common authenticated call needs
to build nothing.

## Auth & middleware: no framework — it's just `try`

There is no middleware abstraction in v1. Auth/session is a plain `task` you `try` at
the top of a handler — `try author = session.user req` short-circuits to a failure if
unauthenticated, composing with the failure-propagation that already exists. This is
the maximally-Pluma answer; cross-cutting concerns (logging, global rate-limit) become
a router-level wrapper *only if* a real need shows up.

## Error model: two channels, deliberately

This is a *convention over existing tools*, not new machinery — both channels already
exist in the language.

- **Transport / infrastructure failures** (network, decode, schema skew, auth-required)
  → the **`task` fails** → handled by the client's `try`/`??`.
- **Domain outcomes** (not-found, validation, "email taken") → a **`wire`'d
  `result a domain-error` *return value*** → the client `when`s over it.

So `fetch` returns `task (result user not-found)`: the `task` channel is transport, the
inner `result` is domain. The point of splitting them:

- They have different *handling* (transport → retry/reconnect/reload UI; domain →
  empty-state / validation UI).
- They have different *producers* (the runtime produces transport failures; the handler
  produces domain results — a handler can't "produce" `network-down`).
- Keeping them apart means a `domain-error` type never gets polluted with
  `network-down` / `client-stale` variants it doesn't own.

At the call site the two are visibly distinct:

```
try outcome = users.fetch (request.new ()) 7   # `try` handles TRANSPORT (propagate / ?? recover)
when outcome is ok u  { render u }              # `when` handles DOMAIN
is err not-found      { show "no such user" }
```

`??` recovers a *transport* failure (`users.fetch … ?? fallback`); `when` handles the
*domain* result. Different tools for different failures.

## Generated plumbing

The user writes zero per-endpoint glue. The compiler:

- collects every `public server def` and emits a **server dispatch table** — route →
  decode args (`from-wire`) → inject `request` → call handler → encode result
  (`to-wire`) — to be mounted on `core.http` (a net-new, server-only module);
- emits **client stubs** — one function per endpoint that encodes args, does the HTTP
  round-trip with the fingerprint header, and decodes the response into a `task`;
- derives the **route** from the qualified name (override possible later).

The server entry just starts `core.http` with the generated dispatcher mounted
(alongside its own routes for serving client assets, JSON endpoints, etc.).

---

# Build & entry points

A fullstack app is a project with **two entry files at its root: `server.pa` and
`client.pa`**. A project with a single **`main.pa`** is an ordinary single program
(script, CLI, or a plain server using `core.http`) — unchanged from today. The presence
of the two entry files is what puts `pluma build` into fullstack mode; mixing `main.pa`
with the pair is an error.

Each entry file holds a `def main` whose expected type fits its role:

- `server.pa`'s `main` starts the HTTP server — it mounts the generated RPC dispatch
  table, serves the client bundle, and adds any of the app's own routes (`core.http`).
- `client.pa`'s `main` is an MVU **`app`** value (Elm-style `init`/`update`/`view`), not a
  run-once `fun` — the runtime drives it. (The `app` type + frontend framework are
  `IR.md` step 2; this doc only assumes the entry exists.)

Everything else is **shared**: any other `.pa` module imported by both. Endpoints
(`public server def`) typically live in those shared modules — the signature is shared,
the body is a server island. Degenerate forms: `client.pa` alone is a pure SPA (a wasm
bundle talking to external APIs, no Pluma server); a server with no Pluma frontend is
just `main.pa`. The paired form is precisely what activates the RPC bridge and the shared
fingerprint.

**Artifacts (note the asymmetry):**

- **Client — mandatory build:** `app.wasm` + a JS loader + an HTML shell. You can't ship
  `.pa` to a browser. Carries the generated client stubs + the fingerprint.
- **Server — a `vm::Program`, run-from-source first:** there's no on-disk `Program` format
  yet, so initially the server is *compiled at launch* (`pluma serve <project>`); a
  serialized-bytecode artifact (faster cold start, ship-without-source) is a deferred
  optimization, not a prerequisite.

**One build, one fingerprint.** `pluma build` computes the schema fingerprint once over the
shared endpoint types and stamps it into *both* artifacts, so client and server can only
agree if built from the same source revision — the build-time teeth behind the
atomic-deploy decision (Layer 1). New CLI verbs: `pluma build`, `pluma serve`, `pluma dev`
(build + serve + watch). `pluma run` is unchanged, for single programs.

# Target-gating

Client-only and server-only stdlib must not leak across the boundary, while shared code
stays portable. Gate **per artifact, by reachability** — no per-module annotation
(consistent with "infer, don't annotate"):

1. Classify stdlib modules in a fixed table: **client-only** (`core.dom`), **server-only**
   (`core.fs`/`core.db`/`core.http`), **portable** (everything else — `list`, `dict`,
   `string`, `math`, `json`, …). A per-module marker can replace the table once packages
   exist.
2. **Client closure** = everything reachable from `client.pa`'s `main` — but traversal
   **stops at `server def` bodies**: it takes the signature (to build the stub) and does
   *not* descend into the body.
3. **Server closure** = everything reachable from `server.pa`'s `main`, bodies and all.
4. **Check:** the client closure must contain no server-only module; the server closure no
   client-only module. Errors name the offending import (`X uses core.db, not available on
   the client`).

The `server def` island is **not a special rule** — it emerges from step 2: because the
client closure never descends into a `server def` body, the `core.db`/`core.fs` calls
inside it are simply never reachable on the client, so they can't trip the check. A
"shared" module is just one that lands in both closures. One mechanism, no tainting.

# Deferred / out of scope here

- **The browser client itself** — WASM/WasmGC backend + the Elm-style MVU frontend
  (`update`/`view`, VDOM/diff, DOM via host imports, `command msg ≈ task msg`). Tracked
  in `IR.md` step 2. This doc assumes it exists and designs the protocol it speaks.
- Middleware framework, default-`request` ergonomics, public/JSON endpoint declaration,
  streaming/subscriptions (server push), file uploads (non-`wire` bodies).

# Open questions

- Exact `wire` binary layout details (varint endianness, float NaN canonicalization,
  string validation) and the `wire-error` variant set.
- What the schema fingerprint hashes precisely, and whether it's per-request or
  negotiated once per session/connection.
- `request` builder ergonomics and whether a metadata-less call can omit it.
- The `server.pa` `main` shape — how the generated RPC dispatch is mounted on `core.http`
  (an explicit value the user mounts vs. implicit injection) and how static-asset serving
  is configured.
- A serialized-bytecode server artifact (an on-disk `vm::Program` format) for faster cold
  start / shipping without source.
- Whether `wire`/`server` belong to core or a `web`/`rpc` package once packages exist.

# Relationship to other docs

- `IR.md` — the IR + the WASM backend that the client target needs (the prerequisite
  for everything client-side here).
- `ASYNC.md` — `task`/`try`/`scope`; the RPC spine and error model are built directly on
  it. A remote call is an async fn; transport failures are task failures.
