# ASYNC.md

Design and implementation plan for Pluma's concurrency story: the
`task a` type, structured concurrency via `scope`/`spawn`, cooperative
cancellation, and `with` blocks for resource cleanup. Builds on the
trait + HKT machinery in [THENABLE.md](THENABLE.md) — `task` becomes a
`thenable`, so the `try` syntax that works for `result` and `option`
works unchanged for async.

Read top-to-bottom for the design; jump to "Implementation phases" to
execute.

## Why

Concrete capabilities this unlocks:

- **Non-blocking I/O.** A web server can handle thousands of in-flight
  requests on one thread; a script can fetch from three APIs at once
  without sequential waiting. Without async, every I/O call blocks the
  whole program.
- **Browser-side Pluma.** The browser is a single-threaded event loop;
  you can't program a browser meaningfully without async primitives.
  This is load-bearing for the fullstack story.
- **Structured cancellation.** Web servers need to drop work when
  clients disconnect; browser apps need to cancel in-flight requests
  when the user navigates. Without first-class cancellation, every
  long-running task becomes a resource leak.

Things we deliberately do NOT include:

- **Multi-core parallelism.** Single-threaded only. JS proves this is
  enough for web work; multi-core can come later as a perf-only
  feature without changing the surface.
- **Pre-emptive scheduling.** Tasks yield at suspension points only.
  CPU-bound pure code can hog a worker; users mitigate with explicit
  `task.yield` if needed.
- **Threads, mutexes, atomics, `SharedArrayBuffer`, COOP/COEP headers.**
  None of it. Single-threaded means none of the multi-threaded
  coordination primitives exist.
- **`async`/`await` keywords.** `task a` is a regular type; `try` over
  it is the existing thenable sugar; suspension is the codegen's
  problem, not the surface's. No function-color marker in the
  signature beyond the return type itself.
- **A web framework, view layer, HTTP server, or runtime-bundled
  ecosystem.** Language primitives only. See "language, not framework"
  in the project values.

## What we're shipping

### `task a` — the deferred-computation type

A `task a` is a value representing a computation that, when run, will
eventually produce an `a` (or be cancelled). Tasks are first-class
values: passable, storable, returnable.

```pluma
let t :: task string = io.read-file "config.json"
let n :: task int    = http.get "/api/count" | task.map string-to-int
```

Tasks are *cold* by default — creating a task value doesn't start it.
A task starts when it's awaited via `try`/`then`, when it's spawned
into a scope, or when explicitly started via `task.start`.

### `for thenable on task`

`task` is an instance of `thenable`, so the existing `try` syntax
works:

```pluma
def fetch-dashboard fun user-id {
    try user    = http.get "/users/$(user-id)"
    try posts   = http.get "/users/$(user-id)/posts"
    try friends = http.get "/users/$(user-id)/friends"
    task.return {user: user, posts: posts, friends: friends}
}
; fetch-dashboard :: fun string -> task dashboard
```

This is sequential — `posts` only starts after `user` resolves. For
concurrent fetches, see "Concurrent composition" below.

### `scope` — structured concurrency

A `scope` is a region of code that owns a set of tasks. A scope can't
exit until every task it owns has either completed or been cancelled.
Cancelling the scope cancels every task it owns, recursively.

```pluma
def fetch-all fun ids {
    scope s {
        let handles = list.map ids fun id {
            s.spawn (http.get "/users/$(id)")
        }
        task.all handles
    }
    ; by the time the scope exits, every spawned task is done or cancelled
}
```

Inside the scope body, `s` is a scope handle. `s.spawn t` adds task
`t` to the scope and returns a `task-handle a` you can await later.
The scope's body itself can use `try` freely.

Sibling cancellation policy: if any spawned task fails (errors, or its
own scope is cancelled), the parent scope cancels its other children
by default. This is configurable per scope.

### Cooperative cancellation

A task observes cancellation at its next suspension point (any
`try`/`then`, any explicit `task.yield`). When cancelled, the task's
continuation is dropped and its `with` cleanups run.

Cancellation propagates *through scopes*, not through individual tasks.
There's no `cancel-token` to thread through every function signature
— the scope is the token, implicit in the call graph.

Three things trigger cancellation:

- **Explicit:** `scope.cancel s` from inside the scope or from a parent.
- **Sibling failure:** another task in the same scope fails (configurable).
- **Parent cancellation:** the parent scope is cancelled.

A task that's been cancelled does *not* observe an `err` value — the
cancellation is structural, not in-band. The scope reports the
cancellation; individual `task a` values just stop producing.

### `with` blocks for resource cleanup

A new statement form. `with x = acquire { body }` runs `acquire`,
binds the result to `x`, runs `body`, and runs the resource's cleanup
when the block exits — whether by success, error, or cancellation.

```pluma
def read-config fun path {
    with f = io.open path {
        try contents = io.read-all f
        parse contents
    }
    ; f is closed here, even if read-all errored or we got cancelled
}
```

Cleanup is dispatched through a `disposable` trait:

```pluma
def disposable trait a {
    dispose :: fun a -> task nothing
}

for disposable on file-handle {
    def dispose f { io.close f }
}
```

Any type with a `disposable` instance can be used in `with`. Stdlib
provides instances for file handles, network connections, and so on.
Users can add their own.

Multiple `with` bindings in one block nest left-to-right, with
cleanups running in reverse order (LIFO):

```pluma
with f = io.open "a.txt"
with g = io.open "b.txt" {
    try a = io.read-all f
    try b = io.read-all g
    ok (a, b)
}
; cleanup order: g closes first, then f
```

### Design alternatives for resource cleanup

**Status: undecided.** The `with`/`disposable` design above is Option
A — the current sketch — but two alternatives are worth keeping in
view. Captured here so the tradeoffs are visible when we pick. All
three rely on the same underlying runtime hook (a per-task cleanup
stack that gets walked on success, error, or cancellation); they
differ only in the surface a user writes.

**Option B — higher-order library helpers.** No new syntax; cleanup
helpers are just functions built on a `runtime.defer-cleanup`
primitive.

```pluma
; single resource — same shape as `with`, no grammar addition
def read-config fun path {
    io.with-file path fun f {
        try contents = io.read-all f
        parse contents
    }
}

; two resources — indent staircase
def diff-files fun (a, b) {
    io.with-file a fun fa {
        io.with-file b fun fb {
            try xa = io.read-all fa
            try xb = io.read-all fb
            ok (compute-diff xa xb)
        }
    }
}

; user-defined helpers: just write a wrapper
def with-pool fun body {
    let p = pool.new ()
    runtime.defer-cleanup fun { pool.drain p }
    body p
}
```

**Option C — `defer` statement (Go / Zig style).** One small grammar
addition; no trait, no helpers required.

```pluma
def read-config fun path {
    let f = io.open path
    defer io.close f
    try contents = io.read-all f
    parse contents
}

def diff-files fun (a, b) {
    let fa = io.open a
    defer io.close fa
    let fb = io.open b
    defer io.close fb
    try xa = io.read-all fa
    try xb = io.read-all fb
    ok (compute-diff xa xb)
}

; user-defined cleanup: nothing special, just defer the call
let p = pool.new ()
defer pool.drain p
try x = pool.borrow p
use x
```

**Comparison:**

| Scenario | A: `with` | B: helpers | C: `defer` |
|---|---|---|---|
| 1 resource | clean | clean | clean |
| N resources | flat, ordered | indent staircase | flat |
| Pair acquire + release visually | yes | yes | weak (two adjacent lines, easy to forget the `defer`) |
| User-defined cleanup | needs `disposable` instance | needs a wrapper helper | nothing — just `defer` the call |
| Conditional acquire (e.g. only if `--log`) | awkward (no else story) | awkward (splits control flow) | trivial (`defer` after the `if`) |
| Cleanup runs on cancellation | yes | yes | yes |
| New grammar | `with` + multi-`with` block rule | none | one statement form |
| New trait | `disposable` (method returns `task`) | none | none |

**Current lean: Option C (`defer`).** Smallest grammar addition, no
new trait, wins on heterogeneous and conditional resources. The
weakness vs. Option A is that nothing forces you to write the
`defer` — `let f = io.open path` with no follow-up compiles fine.
With Option A, forgetting cleanup is structurally impossible. If we
want syntactic enforcement of cleanup, A wins; otherwise C is
cheaper. Option B is the fallback if we don't want to add *any*
syntax for this at all.

### Concurrent composition

`try` chains are sequential. For concurrent composition, stdlib
provides combinators:

```pluma
; run two tasks in parallel, wait for both
try (user, posts) = task.both (http.get "/users/1") (http.get "/users/1/posts")

; run N tasks in parallel
try users = task.all (list.map ids fun id { http.get "/users/$(id)" })

; race — first to complete wins, others are cancelled
try winner = task.race [primary-fetch, fallback-fetch]

; timeout — cancel if it doesn't complete in time
try result = task.with-timeout 5.0s (http.get "/slow-endpoint")
```

These are library code, not language primitives. They build on
`scope`/`spawn` underneath.

## Syntax — the new pieces

### `scope` blocks

```
ScopeBlock ::= 'scope' Identifier '{' BlockBody '}'
             | 'scope' '{' BlockBody '}'
```

The bound identifier (`s` above) is the scope handle. Bare `scope { ... }`
is an anonymous scope — useful when you only need structured awaiting,
not explicit spawning.

The block's body is a regular block: `let`s, `try`s, expressions, the
works. The whole `scope { ... }` is itself an expression that
evaluates to the body's final value.

### `with` bindings

```
WithBinding ::= 'with' Pattern '=' Expr
```

Lives in the same place `let` does — inside a block. Like `try`, it's
a statement form, not an expression. The block desugars by wrapping
the remaining statements in a cleanup-on-exit construct.

Multiple `with`s in one block:

```
with a = ...
with b = ...
{ body }
```

The braces wrap the whole sequence — the `with`s share one body.
Cleanups run in reverse order on exit.

### Task-level operations

These are stdlib calls, not new syntax:

| operation | type |
|---|---|
| `task.return x` | `fun a -> task a` (lift a value into the task carrier) |
| `task.fail err` | `fun e -> task a` (a task that fails) |
| `task.yield ()` | `fun nothing -> task nothing` (give the scheduler a turn) |
| `task.with-timeout d t` | `fun float (task a) -> task a` |
| `task.all ts` | `fun (list (task a)) -> task (list a)` |
| `task.both t1 t2` | `fun (task a) (task b) -> task (a, b)` |
| `task.race ts` | `fun (list (task a)) -> task a` |
| `task.start t` | `fun (task a) -> task-handle a` (eagerly start; rare) |

### Disposable trait

```pluma
def disposable trait a {
    dispose :: fun a -> task nothing
}
```

Standard kind-`*` trait; one method. No HKT needed. Used by `with`.

## Runtime model

### The event loop

Each Pluma program runs in a single OS thread (per process). The
runtime owns:

- A **microtask queue** of tasks ready to resume.
- A **pending-I/O table** of tasks waiting on host I/O completions.
- A **timer wheel** for `task.with-timeout`, `task.sleep`, etc.
- A **scope tree** tracking parent-child relationships.

The loop:

1. While the microtask queue is non-empty, dequeue and run one task
   step (until it suspends or completes).
2. Process completed timers; enqueue their continuations.
3. Block on host I/O if no microtasks are ready; enqueue completions.
4. Exit when: microtask queue is empty, no pending I/O, no live
   timers, and `main` has returned.

### Lazy initialization

The runtime starts only when the first task is created. A script that
never touches `task` or `scope` pays nothing — `main` runs to
completion synchronously and exits. This keeps scripting
ergonomics fast: `pluma my-script.pa` for a hello-world is
indistinguishable from running a sync language.

### Stackless tasks (CPS at codegen)

Each async-bearing function gets a codegen pass that transforms it
into a state machine: a single struct with the function's locals as
fields, a state tag, and one entry point per suspension point. This
is the same shape Rust's `Future` and C#'s `async/await` use.

Suspension is a function return (saving state on the heap); resumption
is a function call (loading state and jumping to the saved state tag).
The VM and WASM target both see synchronous code — there's no fiber
machinery, no stack switching, no special VM instructions.

What identifies an async-bearing function: any function whose body
uses `try` over a `task` carrier, or calls into another async-bearing
function. The transform is contagious; codegen propagates it through
the call graph.

### Host I/O integration

Pluma's runtime doesn't do I/O directly — it asks the host (native or
WASM) to do it and registers a continuation.

- **Native (bytecode VM):** the Rust runtime uses `tokio` (or
  equivalent) under the hood, exposed to Pluma as a single-threaded
  reactor.
- **WASM in browser:** I/O calls compile to imports — `fetch`,
  `setTimeout`, etc. The host JS resumes Pluma's microtask queue when
  promises resolve.
- **WASM via WASI (server):** I/O calls compile to WASI imports —
  `wasi:filesystem`, `wasi:http`. Wasmtime's reactor drives
  completions.

### Cancellation mechanics

A scope owns a `cancel-flag :: ref bool` shared with every task it
spawns. Tasks check this flag at every suspension point. When the
flag is set:

1. The task's state machine returns "cancelled" instead of resuming.
2. The runtime walks the task's `with`-cleanup stack, running each
   `dispose` call (which is itself a task — so the cleanup chain is
   awaited).
3. After cleanup, the task is removed from the scope.
4. When all tasks are removed, the scope exits.

Cleanups during cancellation run on a "best effort" basis: if a
`dispose` task itself errors, the error is logged but doesn't prevent
other cleanups from running. If a `dispose` is *itself* cancelled
(e.g. parent scope timeout fired while cleaning up), we make a hard
choice — currently, cleanup is uninterruptible (cancellation defers
until cleanup completes). Revisit if this becomes a problem.

## Codegen: the CPS transform

The state-machine transform is the largest single piece of new
codegen. Sketch:

```pluma
def fetch-pair fun id {
    try user    = http.get "/users/$(id)"
    try profile = http.get "/users/$(id)/profile"
    task.return (user, profile)
}
```

Compiles roughly to:

```rust
struct FetchPairState {
    state: u8,                  // 0 = start, 1 = waiting on user, 2 = waiting on profile
    id: Value,
    user: Option<Value>,        // populated after state 1
    profile: Option<Value>,     // populated after state 2
    cleanup_stack: Vec<Disposer>,
    scope: ScopeId,
}

fn fetch_pair_step(state: &mut FetchPairState) -> StepResult {
    if state.scope.is_cancelled() { return StepResult::Cancelled; }
    match state.state {
        0 => {
            let fut = http_get(format!("/users/{}", state.id));
            state.state = 1;
            StepResult::Suspended(fut)
        }
        1 => {
            state.user = Some(/* completion */);
            let fut = http_get(format!("/users/{}/profile", state.id));
            state.state = 2;
            StepResult::Suspended(fut)
        }
        2 => {
            state.profile = Some(/* completion */);
            StepResult::Complete(Value::Tuple(vec![state.user.take(), state.profile.take()]))
        }
        _ => unreachable!()
    }
}
```

The bytecode VM and WASM target both see a sequence of synchronous
steps with explicit save/restore points. No coroutine instruction; no
fiber stack.

Key design points:

- **Locals become heap-allocated struct fields** for any function
  containing suspension points. Non-async functions are unaffected.
- **`with` cleanup pushes to the cleanup stack** at the `with` site;
  pops happen at block exit (success) or cancellation walk (failure).
- **Scope membership** is carried in each task's state — needed for
  the cancellation flag check.

## Scripting affordances

Scripts that don't use async pay nothing — the runtime never
initializes. For scripts that *want* async, ergonomics matter:

- `def main = fun { ... }` is the entry point; using `try` inside it
  works the same as in any function. The runtime initializes on first
  task creation, drains, and exits.
- Stdlib provides `*-sync` variants for the most common ops:
  `io.read-file-sync`, `io.write-file-sync`, `process.run-sync`. These
  block the thread; fine for scripts, dangerous for servers (and
  unavailable in the browser target).
- `pluma run script.pa` should feel like `python script.py` or
  `node script.js`. Startup time is a priority.

## Implementation phases

### Phase 1 — `task` type + runtime skeleton

- [ ] Add `task` as a prelude enum constructor (kind `* -> *`,
      represented natively but exposed as if a regular constructor)
- [ ] Add `task.return`, `task.fail`, `task.yield` to prelude as
      hidden VM builtins
- [ ] Add a minimal event-loop runtime to vm/ — microtask queue, run
      loop, exit condition
- [ ] Add `task.sleep` (uses host timer) as the smoke-test I/O op
- [ ] Test fixtures: `task.return 5`, sequential sleep chain

### Phase 2 — `for thenable on task`

Depends on THENABLE.md Phases 1-3 landing first.

- [ ] Declare `for thenable on task` in prelude, dispatching to the
      runtime's bind primitive
- [ ] `try` over task values works
- [ ] Test fixtures: `try` chains on tasks, error propagation,
      mixing-carriers rejection (task + result)

### Phase 3 — CPS transform at codegen

- [ ] Identify async-bearing functions (use `try` over task, or call
      one)
- [ ] Generate state-machine struct per async-bearing function
- [ ] Lower function body into per-state step function
- [ ] Replace direct calls to async-bearing functions with state-
      machine instantiation + first-step call
- [ ] Test: deeply nested async function calls

### Phase 4 — Structured concurrency

- [ ] Parser: `scope IDENT { body }` and `scope { body }` forms
- [ ] AST: `ExprKind::Scope { handle: Option<IdentifierNode>, body }`
- [ ] Runtime: scope tree, cancel-flag, child-task tracking
- [ ] `scope.spawn t` stdlib op, returns task-handle
- [ ] `scope.cancel s` stdlib op
- [ ] Block-on-exit semantics: scope can't return until children done
- [ ] Test fixtures: spawn-and-wait, sibling cancellation,
      nested scopes

### Phase 5 — Resource cleanup

Surface design is undecided — see "Design alternatives for resource
cleanup" above. The runtime side is shared across all three options.

- [ ] Decide between Options A (`with`/`disposable`), B (library
      helpers), and C (`defer` statement)
- [ ] Runtime: per-task cleanup stack; walk it on success, error, and
      cancellation
- [ ] Expose `defer-cleanup` primitive (load-bearing for all three
      options)
- [ ] Per chosen option: parser/AST/codegen for the surface form, or
      prelude trait + instances, or library helpers
- [ ] Test fixtures: cleanup-on-success, cleanup-on-error, cleanup-on-
      cancel, nested resources, heterogeneous resources, conditional
      acquisition

### Phase 6 — Concurrent combinators

- [ ] `task.all` — built on `scope`/`spawn`
- [ ] `task.both`
- [ ] `task.race`
- [ ] `task.with-timeout`
- [ ] Test fixtures for each

### Phase 7 — Sync wrappers and scripting polish

- [ ] `io.read-file-sync`, `io.write-file-sync`, `process.run-sync`
      in `core.io`
- [ ] Verify runtime stays uninitialized for purely-sync scripts
- [ ] Startup-time benchmark; tune

### Phase 8 — WASM backend (post-WASM phase 1)

Depends on a WASM codegen target existing (separate design doc).

- [ ] CPS state machines compile to WasmGC structs
- [ ] Host I/O imports — `fetch`, `setTimeout` for browser; WASI for
      server
- [ ] Same `task`/`scope`/`with` surface, different runtime under the
      hood

## Open questions

1. **`task.return` vs `pure`.**
   `thenable` may grow a `pure` method (see THENABLE.md open question
   #1). If it does, `pure` becomes the carrier-polymorphic
   wrap-a-value; `task.return` becomes a task-specific alias.
   Decide as part of the THENABLE.md `pure` decision.

2. **Eager vs lazy task creation.**
   Above we said tasks are *cold* — creating a task value doesn't
   start it; awaiting does. JS Promises are the opposite (eager). Each
   has tradeoffs. Lazy is more honest (you control when work starts)
   but trips people coming from JS. Decide during Phase 1; lean lazy.

3. **Scope-exit semantics on uncancelled scopes.**
   If `main` returns and the top-level scope has live tasks, do we
   wait for them or cancel them? Python's `trio` waits; this leaks
   work that the user may have forgotten. Tokio's `runtime::block_on`
   waits for the passed future only. Lean: top-level scope cancels
   pending tasks on `main` exit.

4. **Cancellation during cleanup.**
   If a parent scope is cancelled while a `with`-cleanup is running,
   we currently let cleanup finish (uninterruptible). Alternative:
   cancel cleanup too, but provide an "uninterruptible cleanup"
   marker. Decide during Phase 5; lean uninterruptible-by-default.

5. **Should `scope` be implicit in `main`?**
   `main`'s body could be wrapped in an implicit top-level scope so
   `spawn` works without an explicit `scope { ... }`. Saves a level
   of nesting in simple programs but adds magic. Lean: no implicit
   scope — explicit is the convention.

6. **Resource cleanup surface.**
   Three options on the table — `with`/`disposable` syntax (A),
   library helpers over `defer-cleanup` (B), or a `defer` statement
   (C). See "Design alternatives for resource cleanup" above. Lean
   `defer`; the only reason to pick `with` is if we want forgetting
   cleanup to be structurally impossible.

7. **Function color signalling.**
   A function with `try` over `task` in its body returns `task _`.
   The type signature tells the truth, but it's not visually obvious
   at the use site that a call may suspend. Some languages (Kotlin,
   Rust) mark with `suspend`/`async`. Lean: no marker; the `try` (or
   the use of a `task`-returning value) is the visible signal.

---

## Appendix: Worked examples

### C.1 — Sequential fetch with cleanup

```pluma
def save-report fun (report-id, out-path) {
    with f = io.open out-path { write: true } {
        try report = http.get "/reports/$(report-id)"
        try _ = io.write-string f report.body
        ok report.id
    }
}
```

If the HTTP call fails, the file is still closed. If the user
cancels the enclosing scope mid-write, the file is still closed.

### C.2 — Concurrent fetches with timeout

```pluma
def dashboard-data fun user-id {
    task.with-timeout 5.0s {
        try (user, posts, friends) = task.both3
            (http.get "/users/$(user-id)")
            (http.get "/users/$(user-id)/posts")
            (http.get "/users/$(user-id)/friends")
        ok {user, posts, friends}
    }
}
```

All three fetches run in parallel. If any of the three fails, the
others are cancelled. If the whole thing takes more than 5 seconds,
all three are cancelled.

### C.3 — Cancellable long poll

```pluma
def watch-status fun (endpoint, on-change) {
    scope s {
        let prev = ref.new none
        while true is true {
            try current = http.get endpoint
            if (ref.get prev) is some last {
                if last != current.body is true {
                    try _ = on-change current.body
                    ref.set prev (some current.body)
                }
            } else {
                ref.set prev (some current.body)
            }
            try _ = task.sleep 2.0s
        }
    }
}
```

The caller cancels by cancelling the scope (or the parent scope) that
called `watch-status`. The next `try` checkpoint observes the cancel
and unwinds; the loop never gets to run another iteration.

### C.4 — Race with fallback

```pluma
def fetch-with-fallback fun (primary-url, fallback-url) {
    task.race [
        http.get primary-url,
        task.sequence (task.sleep 0.5s) (http.get fallback-url),
    ]
}
```

Try the primary; after 500ms, also try the fallback. Whichever
finishes first wins; the other is cancelled. (`task.sequence` is
"run, then run the second"; lives in stdlib.)

### C.5 — Scripting use (no scope, no async overhead until you ask)

```pluma
def main = fun {
    print "hello"          ; no async — runtime never initializes
}
```

vs.

```pluma
def main = fun {
    try contents = io.read-file "config.json"   ; uses async
    print contents
}
```

The first program runs synchronously and exits immediately. The
second initializes the runtime, suspends on file I/O, resumes when
the read completes, prints, and exits. Same syntax surface, runtime
cost only when actually used.

### C.6 — What this set validates

- **Same `try` syntax across all carriers.** The thenable trait pays
  off — async code reads like sync code modulo the `try` keyword.
- **Cancellation is structural.** No `cancel-token` parameter
  threaded through every function. The scope is the token.
- **Resources can't leak.** The cleanup mechanism (whichever surface
  we pick) runs on every exit path including cancellation.
- **Concurrent composition is library code.** `task.both`,
  `task.race`, `task.with-timeout` build on `scope`/`spawn` — no new
  language primitives.
- **Scripting is unaffected.** Programs that don't use async pay
  nothing for the runtime.
- **The browser story is the same code.** Once the WASM backend
  exists, every example above runs unchanged in a browser.
