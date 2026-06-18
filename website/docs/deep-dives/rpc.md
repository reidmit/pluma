# How RPC works

Most apps just call a `remote def` and get back a value — the network is
invisible. This page is for the curious: how a `remote def` becomes a real HTTP
call, what the compiler generates, and where the boundary between client and
server actually sits.

::: aside .callout
**You don't need this to build apps.** The [Fullstack app](/docs/guides/fullstack)
guide shows the happy path. Read on if you want to understand what the compiler is
doing underneath.
:::

## The `remote def` contract

A `remote def` is the seam between the two halves of a fullstack app. *(Stub —
to be written: how the signature of a `remote def` becomes the wire contract
shared by both sides.)*

## What the compiler generates

From one `remote def` the compiler synthesizes a client stub and a server
dispatch entry. *(Stub — to be written: the generated `rpc-client` / `rpc-server`
code, and how the dual build splits a program into `server.pa` and `client.pa`.)*

## The wire and the transports

*(Stub — to be written: the `wire` codec, the schema-fingerprint skew guard, and
the `http.fetch` / `std/web/fetch` transports the stubs call through.)*

## Ambient context and rejection

*(Stub — to be written: `std/rpc/context`, `rpc.with-headers`, and how a handler
rejects a request with a status.)*
