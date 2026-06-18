# Database access

Pluma talks to an embedded SQLite database through two modules that split the
work cleanly:

- `std/sql` is the **pure** half. It describes queries and decodes rows as plain
  data — it has no idea how to reach a real database, so you can build and inspect
  a query anywhere.
- `std/sys/db` is the **effectful** half. It opens a database file and runs those
  queries. Every operation is asynchronous (it returns a
  [`task`](/docs/reference/concurrency)), and it's a server capability — a
  browser build can't reach it.

## Opening a connection

`db.open` takes a file path, or `":memory:"` for a throwaway in-memory database,
and hands back a connection. Like every db operation it's async, so you await it
with `try`:

```pluma
use std/sys/db

def with-db = fun {
	try conn = db.open "app.db"
	# ... use conn ...
	task.return conn
}
```

## Building a query

You assemble a query by piping builder steps together with
[`|>`](/docs/reference/operators): start `from` a table, then narrow it with
`filter`, `order-by`, `limit`, and so on. The predicates (`sql.eq`, `sql.gt`,
`sql.like`, …) compare a column to a typed value:

```pluma
use std/sql

sql.from "items"
	|> sql.filter (sql.ge "price" (sql.float 2.0))
	|> sql.order-by (sql.desc "id")
```

This is pure data — nothing has run yet. `sql.compile` turns it into the actual
SQL plus a list of values to bind, and that's where the safety guarantee lives:

```pluma
sql.compile (sql.from "users" |> sql.filter (sql.gt "age" (sql.int 17)))
# => ("SELECT * FROM \"users\" WHERE \"age\" > ?", [value.int 17])
```

Notice the `?` — every value becomes a bound parameter, never spliced into the
text, and every identifier is quote-escaped. So a query can't be malformed by its
inputs and **SQL injection is impossible by construction**: a user-supplied value
is always data, never code, because there's no code path that would put it
anywhere else.

## Reading rows

Rows come back untyped — SQLite stores five classes of value (integer, real,
text, blob, null) and decides per cell — so you turn a row into your own record
with a small *decoder*. It pulls each column out by name, and because a column
might be the wrong type or missing, each read returns a
[`result`](/docs/reference/errors) you thread with `try`:

```pluma
use std/sql

alias item {id :: int, name :: string, qty :: option int, price :: float}

def decode-item :: fun row -> result item string = fun r {
	try id = sql.get-int r "id"
	try name = sql.get-text r "name"
	try qty = sql.get-opt r "qty" sql.get-int   # a nullable column
	try price = sql.get-float r "price"
	ok {id, name, qty, price}
}
```

`db.fetch` runs the query and decodes each row, giving back a `task` of your typed
list — the query and the decoder meet at the pipe:

```pluma
use std/sql
use std/sys/db

def dear-items :: fun db.connection -> task (list item) string = fun conn {
	sql.from "items"
		|> sql.filter (sql.ge "price" (sql.float 2.0))
		|> db.fetch conn decode-item
}
```

(SQLite has no boolean storage class, so a bool is stored as 0 or 1 — write one
with `sql.bool` and read it back with `sql.get-bool`.)

## Writing data

For statements that change data, `db.execute` runs raw SQL with a bound parameter
list — the same `?`-and-values shape, so writes are injection-proof too:

```pluma
use std/sql
use std/sys/db

try _ = db.execute conn "insert into items values (?, ?, ?)" [
	sql.int 1,
	sql.text "widget",
	sql.float 2.5,
]
```

`db.transaction` runs several statements as one all-or-nothing unit, and
`db.migrate` applies a numbered list of schema migrations once each. There's also
a higher-level typed-table layer — `db.insert`, `db.update`, `db.delete`, and a
`db.from … |> db.run` query that decodes automatically — when you'd rather work in
terms of a record type than raw columns.

## Errors are values

Every db operation's `task` fails with a plain message rather than throwing, so a
bad statement or a closed connection is something you handle. `task.attempt`
turns a `task` into a `result` when you want to inspect the failure instead of
letting it propagate:

```pluma
try outcome = task.attempt (db.execute conn "select * from nope" [])
# outcome is err "..." rather than crashing the program
```

## See also

- **[Concurrency](/docs/reference/concurrency)** — every query is a `task`;
  `try` awaits it and failures propagate the same way.
- **[Errors and missing values](/docs/reference/errors)** — the `result`s a
  decoder threads, and how a db error erases into `std/error`.
