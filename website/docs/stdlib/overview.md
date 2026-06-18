# The standard library

Pluma ships a standard library covering the everyday needs of a program — data
structures, text, time, JSON, a database, a web stack, a reactive UI. This page is
a map of it. The pages in this section explain the most-used modules in depth;
**every** module, including the ones without a page here, has complete reference
documentation generated from its source at
[the stdlib reference](/std/list).

## Using a module

Every file is a module, and you bring one into scope with `use`, then reach its
contents through the last segment of its path:

```pluma
use std/list

list.map [1, 2, 3] (fun n { n * 2 })   # => [2, 4, 6]
```

A few names are imported for you and need no `use` — `option`, `result`, and
`ref`. Everything else you ask for explicitly. See [Modules](/docs/tour/modules)
for the details.

## The map

**Collections and data**

- [`std/list`](/docs/stdlib/lists) — ordered collections; the workhorse.
- [`std/dict` and `std/set`](/docs/stdlib/dict-set) — key-value maps and unique
  sets.
- [`std/string`](/docs/stdlib/strings) — text: slicing, searching, splitting.
- [`std/bytes`](/docs/reference/bytes) — raw binary data.
- [`std/queue`](/docs/stdlib/queue) — a first-in, first-out queue.
- `std/option` and `std/result` — see [Errors and missing
  values](/docs/reference/errors).

**Numbers and encoding**

- [`std/math`](/docs/stdlib/math) — rounding, roots, logs, trigonometry.
- [Operators](/docs/reference/operators) — arithmetic, comparison, and the
  bitwise operators (`std/bit`).
- `std/base64` and `std/hex` — encoding bytes as text.

**Data formats and text**

- [`std/json`](/docs/stdlib/json) — parsing and writing JSON.
- [`std/regex`](/docs/reference/regex) — the structured regular-expression DSL.
- `std/markdown` — a Markdown parser (these docs are rendered with it).

**Time and identity**

- [`std/time`](/docs/stdlib/time) — the clock, durations, and calendar dates.
- [`std/random` and `std/uuid`](/docs/stdlib/random) — secure randomness and
  unique ids.

**Errors and concurrency**

- [`std/error`](/docs/reference/errors) — the late-erasing error carrier.
- [`std/task`](/docs/reference/concurrency) — asynchronous computations,
  `scope`, and `defer`.

**Building a UI**

- [`std/view`](/docs/stdlib/view) — your interface as a tree of elements.
- [`std/css`](/docs/stdlib/css) — typed styles and scoped classes.
- [`std/signal`](/docs/deep-dives/signals) — the fine-grained reactivity
  underneath.
- `std/keyed` and `std/event` — list keying for `view.each`, and DOM events.

**Server and system**

These are server capabilities — a browser build can't reach them.

- [`std/sys/http`](/docs/stdlib/http) — the HTTP server and client.
- [`std/sql` and `std/sys/db`](/docs/stdlib/database) — typed SQL over SQLite.
- [`std/sys/fs`](/docs/stdlib/files) — files and directories (with `std/path`).
- `std/sys/net` — raw TCP sockets, under the HTTP stack.
- `std/sys/io`, `std/sys/process`, `std/sys/terminal` — standard streams, the
  process environment, and ANSI terminal output.

**Fullstack**

- [`remote def` and RPC](/docs/deep-dives/rpc) — typed server calls; built on
  `std/rpc`, `std/router`, and `std/middleware`.
- [`std/stream`](/docs/stdlib/streams) — pull-based async streams, for
  server-to-client events.

**Testing**

- `std/test` and `std/assert` — write a `*.test.pa` suite and run it with `pluma
  test`. See the [command-line guide](/docs/guides/cli).

## Reading a module's full docs

Wherever a module isn't covered by a page in this section, its source is its
documentation: open [`/std/<module>`](/std/list) — for example `/std/queue` or
`/std/bit` — for every public function with a one-line summary and an example, the
same comments you'd see in your editor's hovers.
