# Server-side rendering

When a fullstack app serves a page, the server renders the view to HTML first, so
the browser shows real content before any code runs. The browser then *hydrates*
that markup — taking over the existing DOM and wiring up reactivity without
rebuilding it. This page explains the round trip.

::: aside .callout
**You don't need this to build apps.** Server rendering is on by default. Read on
if you want to understand the handoff between server and browser.
:::

## Rendering a view to HTML

The same `std/view` tree that runs in the browser also renders to a string on the
server. *(Stub — to be written: the server render path and how it produces the
initial HTML.)*

## Hydration

*(Stub — to be written: how the browser bundle adopts the server-rendered DOM
instead of replacing it, and how signals re-attach to existing nodes. See
[Reactive frontend with signals](/docs/deep-dives/signals).)*

## The dual build

*(Stub — to be written: how one program is compiled into a server binary and a
browser bundle, and what determines which code ends up where. See
[How RPC works](/docs/deep-dives/rpc).)*
