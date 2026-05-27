# ASYNC.md

Design and implementation plan for Pluma's concurrency story: the
`task a` type, structured concurrency via `scope` / `manual scope`,
cooperative cancellation, and `defer` for resource cleanup.

**Status (sequential async ships; structured concurrency does not).**
Built and tested:

- **Phases 1–3 — `task`, `try` over `task`, the CPS transform.** The
  `task` prelude type, the auto-imported `core.task`, `try`/`??` over
  the task carrier, and the state-machine runtime all ship. *Realization
  note:* the CPS transform is implemented as a **resumable step
  function** per async-bearing function plus a runtime driver (`vm::task`,
  the activation-stack event loop), rather than as a separate codegen
  pass emitting an explicit state-struct. The observable semantics
  (cold/lazy, stackless suspension, re-runnable recipes) are exactly as
  designed; the "state tag" is just the saved instruction pointer and
  the heap-saved frame region is the locals struct. The explicit
  state-struct codegen remains a possible later optimization.
- **Phase 5 — `defer` synchronous core**, including across `try`
  suspension points and up a failing await chain.
- **Phase 6 (partial) — the task *transformers*:** `task.return` /
  `fail` / `yield` / `sleep` / `then` / `or-else` / `attempt` / `map`.

Not built yet (this doc is the design of record for them): **Phase 4**
(`scope` / `manual scope` and the scheduler), the **scope-level
combinators** of Phase 6 (`all` / `both` / `race` / `any` / `pool` /
`with-timeout` — they need the `scope` kernel), the async-only facets of
**`defer`** (cancellation cleanup, awaited `task`-returning cleanup,
`task.shielded`), **Phase 7** (sync wrappers), and **Phase 8** (WASM).
`task.once` (run-once memoization) is also still pending.

It builds on the `try` chaining mechanism, which already ships for
`result` and `option`. `try` is a built-in form that dispatches over a
fixed carrier set; `task` becomes a third carrier (add a `task.then`
and a dispatch-table row), so the `try` syntax works unchanged for
async — awaiting adds **no new grammar**. There is no `thenable`
typeclass — that was an earlier design that was dropped in favor of
the built-in form.

The entire feature adds exactly **one** new grammar production — the
`scope` block — plus the small `defer` statement for cleanup.
Everything else (awaiting, cancellation, timeouts, the concurrent
combinators) is the existing `try` form, methods on the scope handle,
or auto-imported `core.task` library functions.

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
  it is the existing `try`-carrier sugar; suspension is the codegen's
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

Tasks are also **re-runnable**: a `task a` is a *recipe*, not a result.
Operationally it's a thunk — a closure that builds a *fresh*
computation each time it's run (in the CPS model, the value is a
factory for state-machine instances). Awaiting or spawning the same
task value twice runs it twice — re-doing any side effects:

```pluma
let t = io.write-file path data
try _ = t   ; writes once
try _ = t   ; writes again — t is a recipe, not "the result"
```

This is what lets `retry`/`repeat`/polling take a plain `task` instead
of forcing callers to wrap in a `fun { ... }` thunk. To run-once and
share the result, opt in with `task.once :: fun (task a) -> task a`
(the JS-Promise behaviour, made explicit). Re-runnable is the strictly
more expressive default — you can always derive memoized from it, but
not the reverse — and it's consistent with the other two `try`
carriers (`option`/`result` values are likewise re-usable).

**`task` vs `task-handle`.** The recipe/instance split is the key
distinction:

- A **`task a`** is the re-runnable recipe (≈ a thunk `() => work`).
- A **`task-handle a`**, returned by `s.spawn`, is one specific
  *running instance* (≈ a started JS promise). Awaiting a handle twice
  yields the same cached result — the run already happened.

So `retry` re-runs a *task*; within one run you await *handles*.

### `try` over `task`

`task` becomes a third `try` carrier alongside `result` and `option`,
so the existing `try` syntax works:

```pluma
def fetch-dashboard = fun user-id {
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
exit until every task it owns has either completed or been cancelled —
this is the *structural guarantee*, and it holds for both scope forms
below. Cancelling the scope cancels every task it owns, recursively.

```pluma
def fetch-all = fun ids {
    scope as s {
        let handles = list.map ids (fun id { s.spawn (http.get "/users/$(id)") })
        await-all handles
    }
    ; by the time the scope exits, every spawned task is done or cancelled
}
```

Inside the scope body, `s` (bound by `scope as s`) is the scope handle.
The handle's operations are **methods on `s`** — not free functions,
not module calls:

| method | meaning |
|---|---|
| `s.spawn t` | start task `t` in the scope; returns a `task-handle a` (hot — already running) |
| `s.next` | (manual scope only) await the next child to settle: `task (option (result a))` |
| `s.cancel` | cancel this scope and all its children |
| `s.cancel-after d` | arm a deadline — self-cancel after duration `d` |

They are methods because `scope` is a keyword, so there's no `scope.`
module namespace to hang a `scope.cancel` off of (see "Module and
prelude structure"). The scope body itself can use `try` freely.

#### Two scope forms: `scope` and `manual scope`

The everyday form is **`scope`** — *fail-fast*. The building-block form
is **`manual scope`**. They share the structural guarantee
(exiting cancels any still-running children) and differ in exactly one
thing:

> **Does an *unobserved* mid-flight child failure trigger a prompt
> background cancel of its siblings?** `scope`: yes. `manual scope`:
> no — you are expected to be draining completions yourself via
> `s.next`.

- **`scope`** (fail-fast) is the safe human default. A child crashing
  promptly cancels its siblings and the failure propagates out as the
  scope's value. This is what 95% of hand-written scopes — and the
  `all` / `both` combinators — want.
- **`manual scope`** never background-cancels on failure. You
  spawn into it, pull completions one at a time with `s.next`, and
  decide what to do. It's the building block under `race` / `any` /
  `pool`. A `manual scope` must bind a handle (`manual scope as s`) —
  an anonymous one is useless, since you couldn't drain it.

#### Why two forms and not a policy matrix

The space of "what happens when a child finishes" is a 2×2 — on-fail ∈
{stop, keep} × on-success ∈ {stop, wait} — and it is exactly JS's four
combinators:

| | on-fail: stop | on-fail: keep |
|---|---|---|
| **on-success: wait** | `all` | `allSettled` |
| **on-success: stop** | `race` | `any` |

We deliberately **do not** expose this as scope configuration, because
the two axes live at different levels and collapse cleanly:

- **The on-fail axis is a property of a *task*, not the scope.** "Keep
  going when this fails" just means "this task doesn't fail" — reify
  the failure into the value channel with
  `task.attempt :: fun (task a) -> task (result a e)`. So `allSettled`
  is just `all` of `attempt`-wrapped tasks. The whole *on-fail: keep*
  column is one task combinator, no scope knob.
- **The on-success axis is genuinely scope-level** — "first one wins,
  cancel the rest" is a statement about siblings that only the scope
  can act on — and it's precisely what `manual scope` + `s.next`
  provide.

So the matrix becomes: fail-fast `scope` (the `all` cell) + `attempt`
(the failure axis) + `manual scope` / `s.next` (the success axis). No
config bag.

### Cooperative cancellation

A task observes cancellation at its next suspension point (any
`try`/`then`, any explicit `task.yield`). When cancelled, the task's
continuation is dropped and its `defer` cleanups run.

Cancellation propagates *through scopes*, not through individual tasks.
There's no `cancel-token` to thread through every function signature
— the scope is the token, implicit in the call graph.

Four things trigger cancellation:

- **Explicit:** `s.cancel` from inside the scope, or from a parent.
- **Sibling failure:** in a fail-fast `scope`, another child failing
  cancels the rest (see above). A `manual scope` does *not* do this.
- **Parent cancellation:** the parent scope is cancelled.
- **Deadline:** the scope was armed with `s.cancel-after d` (directly,
  or by `task.with-timeout`) and the duration elapsed.

A task that's been cancelled does *not* observe an `err` value — the
cancellation is structural, not in-band. The scope reports the
cancellation; individual `task a` values just stop producing.

A region that must survive cancellation (typically cleanup) can be run
uninterruptibly via the `task.shielded` combinator — see "`defer`".

### `defer` — resource cleanup

**Decision: `defer`** (Go / Zig / Swift style), settled over the two
alternatives recorded at the end of this section. `defer expr`
schedules `expr` to run when the enclosing function body exits — by
*any* path: normal return, failure, or cancellation.

```pluma
def read-config = fun path {
    let f = io.open path
    defer io.close f
    try contents = io.read-all f
    parse contents
}
; f is closed here on every path — success, error, or cancellation
```

Multiple `defer`s run in reverse order (LIFO), so acquire/release nest
correctly even across *heterogeneous* resources:

```pluma
def diff-files = fun a b {
    let fa = io.open a
    defer io.close fa
    let fb = io.open b
    defer io.close fb
    try xa = io.read-all fa
    try xb = io.read-all fb
    ok (compute-diff xa xb)
}
; cleanup order: fb closes first, then fa
```

`defer` shines on the cases the alternatives fumble:

- **Heterogeneous resources:** each gets its own `defer`, flat — no
  indent staircase.
- **Conditional acquisition:** a `defer` after an `if` is trivial;
  there's no "else" puzzle.
- **User-defined cleanup:** nothing special — `defer pool.drain p`. No
  trait to implement, no wrapper helper to write.

If a deferred expression returns a `task` (e.g. an async `io.close`),
the cleanup is awaited; the per-task cleanup stack runs the chain on
the way out. Cleanup during cancellation runs **uninterruptibly** by
default — a parent cancel won't abort an in-progress cleanup. Wrap a
region in `task.shielded` if you need to extend that guarantee
explicitly.

**Anchoring:** `defer` is tied to the enclosing **function** body
(Go's model), not the nearest block — the most familiar and
predictable choice. *Confirmed at implementation:* cleanups are
anchored to the VM **frame**, which is exactly the function body —
`if`/`when`/`while` blocks compile inline into the same frame, so a
`defer` inside them still fires at function exit. The one wrinkle is
that a `try` rewrites its continuation (everything textually after it)
into a separate closure/frame, so a `defer` placed *after* a `try`
rides that continuation's frame. This is invisible: the continuation's
Return runs as part of producing the function's return value, so the
LIFO order and the "only fires if its `defer` was reached" rule both
hold exactly as if anchored to one flat function body. (A consequence:
a `defer` after a `try` is skipped when that `try` short-circuits —
which is the correct, intended behavior.)

**Runtime:** `defer expr` pushes a cleanup thunk onto the task's
cleanup stack at the point the `defer` executes; the stack is walked
LIFO on success, error, and cancellation. This is the same shared
runtime hook all three candidate surfaces would have used.

#### Alternatives considered (rejected)

- **`with x = acquire { body }` + a `disposable` trait.** Makes
  forgetting cleanup structurally impossible, but costs new grammar
  (the with-binding *plus* a multi-`with` block-nesting rule) *and* a
  new trait, and still fumbles conditional/heterogeneous resources.
  Rejected: too much surface for the enforcement win, and against the
  minimal-grammar grain of the rest of this design.
- **Library helpers over a `runtime.defer-cleanup` primitive** (e.g.
  `io.with-file path (fun f { ... })`). Zero grammar, but produces an
  indent staircase for multiple resources, is awkward for conditional
  acquisition, and exposes `runtime.defer-cleanup` to users anyway.
  Rejected: `defer` is barely more grammar and far more ergonomic.

`defer` won because it's one tiny statement (no trait, no new type),
maximally familiar, and the only option that stays flat across
heterogeneous and conditional resources. Its sole weakness — nothing
*forces* you to write the `defer` — is the price of not adding the
heavier `with` machinery.

### Concurrent composition

`try` chains are sequential. Concurrent composition is **library code**
over the primitive kernel below — not new language primitives:

```pluma
; run two tasks in parallel, wait for both
try (user, posts) = task.both (http.get "/users/1") (http.get "/users/1/posts")

; run N tasks in parallel
try users = task.all (list.map ids (fun id { http.get "/users/$(id)" }))

; first to settle wins; the losers are cancelled
try winner = task.race [primary-fetch, fallback-fetch]

; bounded concurrency — at most 8 of these run at once
try results = task.pool 8 (list.map urls (fun u { http.get u }))

; timeout — a deadline'd scope; the work is actually cancelled on expiry
try result = task.with-timeout 5.0s (http.get "/slow-endpoint")
```

#### The primitive kernel

Everything above is written in Pluma over a small kernel:

- `s.spawn t` — start a task in a scope, get a hot handle
- `try h` — await a handle (the existing `try`, reused)
- `s.cancel` / `s.cancel-after d` — cancel now / on a deadline
- `s.next` — drain the next completion (manual scopes)
- two scope forms — `scope` (fail-fast) and `manual scope`
- `task.attempt t` — reify a task's failure into `result` (carries the
  on-fail policy axis)

`all`, `both`, and `settle-all` need only `spawn` + `try` + the
fail-fast default. `race`, `any`, `pool`, and `with-timeout` are the
ones that need `manual scope` + `s.next` — the single primitive an
earlier draft of this doc was missing (it claimed these were "built on
scope/spawn", but `spawn` + await-a-specific-handle can't express
"whichever finishes first"). Worked implementations are in Appendix
C.7.

## Syntax — the new pieces

### The `scope` block (the only new concurrency grammar)

```
ScopeExpr ::= 'scope' ('as' Identifier)? Block            ; fail-fast; handle optional
            | 'manual' 'scope' 'as' Identifier Block       ; manual; handle required
```

The handle is bound with `as`, reusing the same keyword `use core.x as
y` already uses to introduce a name — so `scope as s` reads "make a
scope, call it `s`", consistent with the rest of Pluma. The manual form
is spelled with a `manual` *prefix* modifier, matching the
`public def` / `opaque enum` prefix-modifier shape Pluma already uses
(not a trailing modifier, which Pluma has nowhere). `scope` is a
keyword; `manual` is a *contextual* keyword (special only immediately
before `scope`). The block is an **expression** — it evaluates to its
body's final value and slots in anywhere `if` / `when` can; bare
`scope { ... }` is an anonymous fail-fast scope (useful when you only
need structured awaiting, not explicit spawning).

That is the entire grammar addition for *concurrency*. In particular:

- **Awaiting** reuses the existing `try` (and `??`) — no new syntax.
- **Handle operations** (`s.spawn`, `s.next`, `s.cancel`,
  `s.cancel-after`) are method calls on the handle value, resolved by
  its type — not grammar.
- **`deadline` and `shield` are deliberately not grammar.** A deadline
  is the `s.cancel-after` method (or the `task.with-timeout`
  combinator); shielding is the `task.shielded` combinator. Both were
  considered as scope clauses and demoted to keep the grammar to the
  single production above.

### The `defer` statement (the only new cleanup grammar)

```
DeferStmt ::= 'defer' Expr
```

Lives where `let` does — inside a block. See "`defer`" above for
semantics (runs on exit by any path, LIFO, function-anchored).

### Task-level operations (stdlib, not syntax)

All live in the auto-imported `core.task` module (see "Module and
prelude structure"). Carrier essentials:

| operation | type |
|---|---|
| `task.return x` | `fun a -> task a` (lift a value into the carrier) |
| `task.fail e` | `fun e -> task a` (a task that fails) |
| `task.yield ()` | `fun nothing -> task nothing` (give the scheduler a turn) |
| `task.attempt t` | `fun (task a) -> task (result a e)` (reify failure) |
| `task.once t` | `fun (task a) -> task a` (run at most once, cache/share the result) |
| `task.map f t` | `fun (fun a -> b) (task a) -> task b` |
| `task.or-else t f` | recover from failure (what `??` desugars to over `task`) |
| `task.sleep d` | `fun duration -> task nothing` |
| `task.start t` | `fun (task a) -> task-handle a` (eagerly start; rare) |

Concurrent combinators, all built on the kernel (see Appendix C.7):

| operation | type |
|---|---|
| `task.all ts` | `fun (list (task a)) -> task (list a)` |
| `task.both t1 t2` | `fun (task a) (task b) -> task (a, b)` |
| `task.settle-all ts` | `fun (list (task a)) -> task (list (result a e))` |
| `task.race ts` | `fun (list (task a)) -> task a` |
| `task.any ts` | `fun (list (task a)) -> task a` |
| `task.pool n ts` | `fun int (list (task a)) -> task (list a)` |
| `task.with-timeout d t` | `fun duration (task a) -> task a` |
| `task.retry n t` | `fun int (task a) -> task a` |
| `task.shielded t` | `fun (task a) -> task a` (uninterruptible) |

### Module and prelude structure

`task` mirrors how `option` and `result` already work — two
mechanisms, deliberately layered:

- **The `task` type is prelude.** Like the `option`/`result` enums in
  `prelude.pa`, the `task` constructor (kind `* -> *`, represented
  natively) is always in scope. You write `task string` with no import.
- **The `task.*` functions live in `core.task`, auto-imported.** They
  go in `AUTO_IMPORTS` (compiler.rs) alongside `core.option` and
  `core.result`, so `task.all` resolves with no `use core.task` — the
  same way `option.map` works today. FieldAccess dispatch handles the
  overlap between the prelude type and the module namespace, exactly as
  it does for `option`.

The principle: **the three `try`-carriers — `option`, `result`,
`task` — are the auto-imported modules; everything else (`core.list`,
`core.dict`, `core.string`, …) needs an explicit `use`.** `task` is
auto-imported *because* it's a carrier, which also keeps scripting
ceremony-free (async needs no import line).

`scope` is a different category entirely: it's a **keyword** (grammar),
not a value or a module. There is no `use core.scope` and no prelude
*binding* named `scope` — the parser just knows it, like `if` / `when`
/ `try`. Consequently the scope-handle operations can't be
`scope.cancel`-style module calls (`scope.` is a keyword prefix); they
are methods on the handle value.

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
2. The runtime walks the task's `defer`-cleanup stack (LIFO), running
   each deferred expression (which may itself be a task — so the
   cleanup chain is awaited).
3. After cleanup, the task is removed from the scope.
4. When all tasks are removed, the scope exits.

Cleanups during cancellation run on a "best effort" basis: if a
deferred cleanup itself errors, the error is logged but doesn't prevent
other cleanups from running. If a cleanup is *itself* cancelled (e.g.
parent scope deadline fired while cleaning up), we make a hard
choice — currently, cleanup is uninterruptible (cancellation defers
until cleanup completes), which is what `task.shielded` makes explicit.
Revisit if this becomes a problem.

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
- **`defer` pushes to the cleanup stack** at the `defer` site; the
  stack is walked LIFO on block/function exit (success) or the
  cancellation walk (failure).
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

- [x] Add `task` as a prelude type constructor (kind `* -> *`,
      represented natively but exposed as if a regular constructor) — a
      variant-less prelude enum `task a`.
- [x] Create `core.task` and add it to `AUTO_IMPORTS` (compiler.rs)
      next to `core.option`/`core.result`, so `task.*` resolves with
      no `use`
- [x] Add `task.return`, `task.fail`, `task.yield` as hidden VM
      builtins exposed through `core.task`
- [x] Add a minimal event-loop runtime to vm/ (`vm/src/task.rs`): an
      activation-stack driver run lazily when `main` returns a task.
      (A multi-fiber scheduler/microtask queue arrives with Phase 4.)
- [x] Add `task.sleep` (host timer) as the smoke-test I/O op
- [x] Represent a `task` as a re-runnable factory (a cold `TaskRepr`
      recipe; `Async` instantiates a fresh frame per run), not a
      one-shot value. (`task.once` for memoized runs still pending.)
- [x] Test fixtures: `task.return`, sleep chain (`tests/run/task-*`)

### Phase 2 — `try` over `task`

The `try` mechanism this builds on has already shipped for
`option`/`result`.

- [x] Add a `task` row to the analyzer's `try` dispatch table. Unlike
      option/result, a task `try` is **not** rewritten to `task.then` —
      it's flagged (`TryNode.task_carrier`) and left intact so codegen
      can lower the whole chain into a state machine (that closure-tree
      *is* the trampoline we're avoiding).
- [x] `try` over task values works; `??` over task → `task.or-else`
- [x] Test fixtures: `try` chains on tasks, error propagation
      (`tests/run/task-*`), mixing-carriers rejection
      (`tests/analyze/task-mixed-carriers`)

### Phase 3 — CPS transform at codegen

Realized as resumable step functions + a runtime driver (see the status
note at the top), not an explicit state-struct codegen pass. Same
observable semantics.

- [x] Identify async-bearing functions — `body_is_async` in codegen
      (a `try`-over-task in the function's own frame). Decided locally;
      no whole-program contagion needed.
- [x] Lower the body into a resumable step function: each `try` →
      evaluate-task + `Await`, continuation emitted inline; the whole
      frame region is heap-saved at each `Await` and restored on resume
      (so awaits may occur mid-expression).
- [x] Calling an async function builds a cold `Value::Task` recipe
      (`do_call`'s `AsyncFn` arm) — the "instantiation"; the driver
      first-steps it. No call-site knowledge of async-ness needed.
- [x] Test: deeply nested async calls, async closures with captures,
      defer across suspension (`tests/run/task-*`)

### Phase 4 — Structured concurrency

- [ ] Parser: `scope (as IDENT)? { body }` and `manual scope as IDENT
      { body }` (the single production; `manual` as a contextual prefix
      keyword; handle bound with the existing `as`)
- [ ] AST: `ExprKind::Scope { manual: bool, handle: Option<IdentifierNode>, body }`
- [ ] Runtime: scope tree, cancel-flag, child-task tracking
- [ ] Handle methods on the scope value: `s.spawn t` (→ task-handle),
      `s.cancel`, `s.cancel-after d`
- [ ] `s.next` for manual scopes: drain the next completion
- [ ] Fail-fast vs manual: an unobserved child failure prompt-cancels
      siblings in `scope` but not in `manual scope`
- [ ] Block-on-exit semantics: scope can't return until children done
- [ ] Test fixtures: spawn-and-wait, fail-fast sibling cancellation,
      `manual scope` + `s.next` draining, deadline, nested scopes

### Phase 5 — Resource cleanup (`defer`)

Surface is **decided: `defer`** (see "`defer`"). The synchronous core
landed ahead of the rest of the async machinery — it's useful on its
own and the runtime hook generalizes when scopes/cancellation arrive.

- [x] Parser: `DeferStmt ::= 'defer' Expr` (function-anchored). `defer`
      is a body statement (dispatched like `let`), evaluating to
      `nothing`; missing operand is a parse error.
- [x] Runtime: per-**frame** cleanup stack, walked LIFO on Return —
      covering normal return *and* `try`-failure propagation (the
      short-circuited `err`/`none` still flows through the frame's
      Return). Codegen lowers `defer expr` to a zero-arg thunk
      (`fun { expr }`) pushed via a new `PushDefer` op; the thunk
      captures referenced locals by value at the `defer` site. A frame
      with pending cleanups opts out of tail-call reuse at runtime (so
      its Return actually executes); a raising cleanup propagates and
      skips the remaining cleanups (revisit when cancellation lands).
- [ ] Cancellation cleanup + uninterruptible-by-default — needs the
      `task` runtime (Phases 1–4).
- [ ] Awaited cleanup: a `defer` whose expression returns a `task`.
- [ ] `task.shielded` combinator for explicit uninterruptible regions.
- [x] Test fixtures: cleanup-on-success, cleanup-on-`try`-failure,
      conditional acquisition, LIFO ordering, raising cleanup
      (`tests/run/defer-cleanup`; `tests/analyze/defer`,
      `tests/analyze/defer-no-operand`; `tests/format/defer`).
- [ ] Test fixtures still pending: cleanup-on-cancel, async cleanup.

### Phase 6 — Concurrent combinators

The scope-level combinators are built on the Phase 4 kernel (`scope` /
`manual scope` / `s.next`); the task-level transformers don't need it.
See Appendix C.7 for implementations.

- [x] `task.attempt` (reifies failure — carries the on-fail axis), plus
      `task.then` / `task.or-else` / `task.map` (the transformers) —
      builtins building `TaskRepr` nodes, interpreted by the driver.
- [ ] `task.all`, `task.both`, `task.settle-all` (fail-fast scope)
- [ ] `task.race`, `task.any`, `task.pool` (`manual scope` + `s.next`)
- [ ] `task.with-timeout` (deadline'd scope), `task.retry`
- [x] Test fixtures for the transformers (`tests/run/task-combinators`);
      scope-level combinators pending Phase 4.

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
- [ ] Same `task` / `scope` / `defer` surface, different runtime under
      the hood

## Resolved decisions

These were open and are now settled (this design round):

1. **Type name: `task`** (kept). `promise` rejected — it implies eager
   semantics (a JS Promise starts on creation), which would mislead
   given the cold/lazy lean below. `future` was runner-up (Rust's lazy
   future is the closest semantic match) but `task` reads better with
   the structured-concurrency vocabulary ("spawn a task into a scope").

2. **Eager vs lazy: lazy/cold.** Creating a `task` does nothing;
   awaiting (`try`), spawning, or `task.start` runs it. This is what
   makes tasks first-class plans you can store, pass, retry,
   rate-limit, and conditionally run — things eager JS Promises can't
   express without dropping to thunks.

3. **Scope policy: two named forms, no config matrix.** `scope`
   (fail-fast) and `manual scope`, plus `task.attempt` for the
   on-fail axis. See "Why two forms and not a policy matrix".

4. **Resource cleanup: `defer`** (was Option C). See "`defer`".

5. **Timeouts are deadline'd scopes**, not races against a sleeper —
   `s.cancel-after` / `task.with-timeout`. Deadline is the fourth
   cancellation trigger.

6. **Minimal grammar:** the whole feature adds the `scope` block
   production plus the `defer` statement. `deadline`/`shield` are
   methods/combinators, not grammar.

7. **Function color.** A `try`-over-`task` function returns `task _`;
   that *is* the color, spelled in the return type. For a typed FP
   language this is a feature (an honest effect annotation), not a
   wart — no `async`/`suspend` marker. Revisit only if it bites.

8. **Re-runnable, not one-shot.** A cold `task a` is a re-runnable
   *recipe* (operationally a thunk that builds a fresh computation each
   run), not a one-shot consumed-on-poll value. This lets `retry`/
   `repeat`/polling take a plain `task`, is the strictly more
   expressive default (`task.once` opts into memoization; you can't go
   the other way), and is consistent with the other two re-usable
   carriers. The running instance you get from `s.spawn` is a
   `task-handle`, which *is* one-shot (awaiting it twice yields the
   cached result). Reconfirms the name `task` over `future`. See
   "`task a` — the deferred-computation type".

## Open questions (still undecided)

A. **`task.return` naming.** Keep it task-specific vs. a
   carrier-polymorphic `pure`. Lean: keep `task.return` (a shared
   `pure` would need built-in dispatch, since `try` isn't a typeclass).

B. **Scope-exit semantics on `main`.** If `main` returns with live
   tasks, wait or cancel? `trio` waits (can leak forgotten work);
   Tokio's `block_on` waits for the passed future only. Lean: top-level
   scope cancels pending tasks on `main` exit.

C. **Should `scope` be implicit in `main`?** Wrapping `main`'s body in
   an implicit top-level scope lets `spawn` work without an explicit
   `scope { ... }`. Saves nesting but adds magic. Lean: no — explicit
   `scope` is the convention.

---

## Appendix: Worked examples

### C.1 — Sequential fetch with cleanup

```pluma
def save-report = fun report-id out-path {
    let f = io.open out-path { write: true }
    defer io.close f
    try report = http.get "/reports/$(report-id)"
    try _ = io.write-string f report.body
    ok report.id
}
```

If the HTTP call fails, the file is still closed. If the user
cancels the enclosing scope mid-write, the file is still closed.

### C.2 — Concurrent fetches with timeout

```pluma
def dashboard-data = fun user-id {
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
def watch-status = fun endpoint on-change {
    scope as s {
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
def fetch-with-fallback = fun primary-url fallback-url {
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

- **Same `try` syntax across all carriers.** `task` is a third `try`
  carrier — async code reads like sync code modulo the `try` keyword.
- **Cancellation is structural.** No `cancel-token` parameter
  threaded through every function. The scope is the token.
- **Resources can't leak.** `defer` runs on every exit path including
  cancellation.
- **Concurrent composition is library code.** `task.both`,
  `task.race`, `task.with-timeout` build on the `scope` / `manual
  scope` / `s.next` kernel — no new language primitives.
- **Scripting is unaffected.** Programs that don't use async pay
  nothing for the runtime.
- **The browser story is the same code.** Once the WASM backend
  exists, every example above runs unchanged in a browser.

### C.7 — The combinators, implemented

Proof that the concurrent combinators are ordinary Pluma over the
kernel. A shared helper:

```pluma
; settled result -> task carrier
def result-to-task = fun r {
    when r is ok x { task.return x } is err e { task.fail e }
}
```

**`both` / `all`** — fail-fast `scope`, spawn + await. Both tasks start
at `spawn`, so awaiting them in order still runs them concurrently; if
either fails, the fail-fast scope cancels the rest and propagates.

```pluma
def both = fun t1 t2 {
    scope as s {
        let h1 = s.spawn t1
        let h2 = s.spawn t2
        try a = h1
        try b = h2
        task.return (a, b)
    }
}

def all = fun ts {
    scope as s {
        let handles = list.map ts (fun t { s.spawn t })
        await-all handles
    }
}

def await-all = fun handles {
    when handles is [] { task.return [] }
       is [h, ...rest] {
           try x  = h
           try xs = await-all rest
           task.return [x, ...xs]
       }
}
```

**`settle-all`** — `all` of `attempt`-wrapped tasks. Nothing "fails"
from the scope's view, so fail-fast never fires; you get every outcome.

```pluma
def settle-all = fun ts {
    all (list.map ts (fun t { task.attempt t }))
}
```

**`race`** — `manual scope` + `s.next`. Start everything, take the
first to settle, cancel the losers.

```pluma
def race = fun ts {
    manual scope as s {
        list.map ts (fun t { s.spawn t })
        try first = s.next
        s.cancel
        when first is some r { result-to-task r }
                   is none   { task.fail "race: empty" }
    }
}
```

**`with-timeout`** — composes on `race`: the work versus a sleeper that
fails. If the timer wins, the work is cancelled.

```pluma
def with-timeout = fun d t {
    race [t, task.sleep d | task.then (fun _ { task.fail "timeout" })]
}
```

**`retry`** — no scope at all; pure sequential re-running (works
because tasks are cold and re-runnable — awaiting `t` again runs it
fresh).

```pluma
def retry = fun n t {
    if n <= 1 is true { t }
    else { t | task.or-else (fun { retry (n - 1) t }) }
}
```

**`pool`** — bounded concurrency: prime `n`, then refill from a backlog
each time `s.next` reports a completion.

```pluma
def pool = fun n ts {
    manual scope as s {
        let backlog = ref.new (list.drop n ts)
        list.map (list.take n ts) (fun t { s.spawn t })
        gather s backlog []
    }
}

def gather = fun s backlog acc {
    try got = s.next
    when got is none { task.return (list.reverse acc) }
       is some r {
           try x = result-to-task r        ; a failure here propagates out
           when (ref.get backlog) is [next, ...rest] {
               ref.set backlog rest
               let _ = s.spawn next         ; a slot freed → start one more
               gather s backlog [x, ...acc]
           } is [] {
               gather s backlog [x, ...acc]
           }
       }
}
```
