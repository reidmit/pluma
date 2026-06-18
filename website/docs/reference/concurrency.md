# Concurrency

Pluma does asynchronous work without an `async` or `await` keyword. The unit is a
`task`: a computation that may pause — waiting on a timer, a network reply, a
database — and let other work run while it waits. Tasks are *cooperatively
scheduled* on a single thread, so nothing runs in parallel and you never juggle
locks; a task makes progress until it suspends at a waiting point, and then a
sibling gets its turn.

This page covers writing a task, awaiting one, running several at once, and
bounding their lifetimes with a scope. The `task` type and the async syntax
(`try`, `scope`, `defer`, duration literals) are always available; the named
functions live in `std/task`.

## A task, and awaiting it

A function that does async work returns a `task`. Its type is `task a e` — a task
that, when it finishes, either produces an `a` or fails with an `e`. (That's the
same success/failure split as `result`, carried through an asynchronous
computation.) You build the simplest tasks with `task.return`, which wraps a
finished value, and `task.fail`, which fails:

```pluma
use std/task

def fetch-port :: fun nothing -> task int string = fun {
	task.return 8080
}
```

Inside a task-returning function, `try` *awaits* another task: it runs it,
waits for the result, and binds the success — or short-circuits with the failure,
exactly like `try` over a `result`. So async code reads top to bottom, with no
callbacks and no special await syntax:

```pluma
def load-greeting :: fun string -> task string string = fun url {
	try page = http.get url       # await; on failure, bail out
	try name = field page "name"
	task.return "hi, $(name)"
}
```

This is the whole reason tasks exist as their own type: a sequence of awaits
reads like ordinary straight-line code, and the failure path is implied. The
[Errors and missing values](/docs/reference/errors) page covers the `try`/`??`
mechanics in full — they work the same over a task as over a `result`, and a
precise failure erases into `std/error` the same way.

## Running tasks concurrently

Awaiting tasks one after another runs them in sequence. To run several at the
*same* time and wait for all of them, hand a list to `task.all`:

```pluma
use std/list
use std/task

def double = fun n {
	try task.sleep 5ms
	task.return (n * 2)
}

try results = task.all (list.map [1, 2, 3] double)
# results => [2, 4, 6]   — the three sleeps overlap
```

The three `double` tasks all suspend on their timers together, so the whole batch
takes about one 5ms nap rather than three. A few more combinators in `std/task`
cover the common shapes:

| Call | What it does |
| --- | --- |
| `task.all ts` | Run every task; succeed with the list of results, or fail as soon as one fails |
| `task.race ts` | Run every task; finish with the first one to settle |
| `task.both a b` | Run two tasks (of possibly different types) and pair their results |
| `task.with-timeout d t` | Run `t`, but fail if it hasn't finished within the duration `d` |
| `task.retry n t` | Re-run `t` up to `n` times while it keeps failing |

## Durations: sleep and timers

A duration is written as a literal: `5ms`, `200ms`, `30s`, `2m20s` (two minutes
and twenty seconds). `task.sleep` suspends just the current task for that long,
on a timer, so siblings keep running while it naps:

```pluma
try task.sleep 250ms   # pause this task a quarter second
```

For a long stretch of CPU-bound work that never naturally suspends, `task.yield
()` hands the scheduler a turn voluntarily, so other tasks — and cancellation —
get a chance to act. Without it, a tight compute loop would hold the thread until
it finished, since nothing is preempted.

## Structured concurrency: scope

`task.all` runs a batch and waits for it. When you want finer control — fire off
some background work, await pieces of it, and guarantee none of it outlives the
block — use a `scope`. Inside, `spawn` starts a task running, and the scope won't
be left until everything it spawned has finished or been cancelled:

```pluma
scope as s {
	let a = s.spawn (client addr "/a")
	let b = s.spawn (client addr "/b")
	try ra = a              # await one of the spawned tasks
	try rb = b
	print (ra ++ " and " ++ rb)
	s.cancel ()             # stop anything still running
	task.return 0
}
```

The promise a scope makes is the useful part: a spawned task can't escape its
scope. When the block ends — normally, by failure, or by `s.cancel ()` — every
task it started is already done or stopped. No task leaks out to run against
freed-up state, which is what makes concurrent Pluma safe to reason about.

## defer: cleanup on the way out

Often a task — or any function — acquires something it must release: an open
connection, a lock, a temp file. `defer` schedules a piece of cleanup to run when
the enclosing function exits, *by any path* — a normal return, or a `try` that
bailed out early:

```pluma
def with-connection = fun addr {
	try conn = connect addr
	defer close conn          # runs however this function exits
	try reply = send conn "ping"
	task.return reply
}
```

`close conn` runs whether `send` succeeds or fails, so you write the cleanup once,
right next to the thing it cleans up, instead of repeating it on every exit.
Several `defer`s run last-registered-first, and a `defer` inside a branch only
fires if that branch actually ran. `defer` isn't async-only — it works in any
function — but it's indispensable once failures and cancellation can cut a task
short.

## See also

- **[Errors and missing values](/docs/reference/errors)** — `try` and `??`, which
  drive a task's success and failure paths.
- **[How RPC works](/docs/deep-dives/rpc)** — `remote def`, where a server call
  becomes a `task` the browser awaits.
