# Reactive frontend with signals

Pluma's frontend is built on fine-grained reactivity: a *signal* holds a value,
and anything that reads it re-runs when it changes. There's no virtual DOM and no
update loop — the framework tracks exactly which views depend on which data and
updates only those. This page explains how that works.

::: aside .callout
**You don't need this to build apps.** The view layer reads naturally without
knowing the internals. Read on if you want to understand why it updates the way it
does.
:::

## Signals and tracking

A signal is a cell you can read and write; reading one inside a reactive context
records a dependency. *(Stub — to be written: `std/signal`, automatic dependency
tracking, and the glitch-free pull model.)*

## The owner tree

Reactive computations are arranged in an ownership tree so that disposing a
parent cleans up its children. *(Stub — to be written: owners, scopes, and
cleanup.)*

## Rendering without a virtual DOM

*(Stub — to be written: how `std/view` binds signals directly to DOM nodes, and
how `view.dyn` / `view.each` re-render only the parts that change.)*

## SSR and hydration

*(Stub — to be written: how the same view renders to HTML on the server and then
takes over reactivity in the browser. See [Server-side rendering](/docs/deep-dives/ssr).)*
