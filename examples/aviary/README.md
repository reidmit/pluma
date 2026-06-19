# Aviary — a fullstack field logbook

A small bird-sighting tracker that exercises Pluma's whole fullstack stack in one
app: **SQLite** on the backend, **RPC** between the halves, **CSS + HTML** on the
frontend, and **three pages, each server-side rendered and then hydrated** in the
browser.

```
pluma dev examples/aviary        # http://localhost:7777
```

(or `pluma build examples/aviary` for the deployable `server.wasm` + client bundle.)

## The pages

- `/` **Logbook** — every sighting, newest first, plus a form to log a new one.
- `/species` **Field Guide** — a per-species tally (a server-side `GROUP BY`),
  drawn as a CSS bar chart.
- `/sightings/{id}` **Detail** — one sighting and its notes, with a form to add a
  note.

Each is rendered on the server with real data for an instant first paint, then the
client adopts that exact DOM (`render.hydrate`) and wires up interactivity. The
server embeds the page's data in a `pluma-boot` payload, so the client hydrates from
it directly — **no refetch round-trip** (`render.boot-data`). In-app links navigate
without a full reload (`history.pushState`), and the browser's back/forward buttons
stay in sync (`popstate`); a direct visit or reload to any URL is still fully
server-rendered.

## How it's wired

| File         | Role |
|--------------|------|
| `model.pa`   | The records that cross the wire + the `page-data` the views render. Shared. |
| `route.pa`   | URL ⇄ `route` parsing. Shared by server (`req.path`) and client (`dom.path`). |
| `db.pa`      | Server-only SQLite layer: schema, seed, queries. Pruned from the web build. |
| `api.pa`     | The `remote def` contract. Real handlers on the server, transport stubs on the client. |
| `ui.pa`      | Isomorphic views (pure `view` builders) + the extracted CSS. Runs on both ends. |
| `server.pa`  | HTTP server: SSR each route, dispatch `/_rpc/*`. |
| `client.pa`  | Reads the URL, fetches the page's data, hydrates, then drives navigation. |

The key split: database access lives only inside `remote def` bodies (`api.pa` →
`db.pa`). On the client those bodies become network stubs, so `std/sys/db` is never
reached and is dropped from the browser build — one program, two artifacts. The
shared `ui.pa` never imports `std/web/dom`; navigation is injected as a callback so
the same views compile into the server artifact, where the DOM doesn't exist.
