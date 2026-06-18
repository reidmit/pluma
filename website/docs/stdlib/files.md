# Files and the filesystem

`std/sys/fs` reads, writes, and manages files and directories. It's a server
capability — a browser build can't reach the disk — and it shares the async,
failures-as-values shape of the rest of the standard library.

## Async by default, with a sync twin

Touching a disk can be slow, so by default every operation is **asynchronous**: it
returns a [`task`](/docs/reference/concurrency), and a slow read parks just the
calling fiber on a worker thread while every other fiber keeps running. You await
the result with `try`, the same as any task.

Each operation also has a synchronous `-sync` twin — `fs.read-text` and
`fs.read-text-sync`, `fs.write-text` and `fs.write-text-sync`. The sync version
blocks the current thread and returns a plain [`result`](/docs/reference/errors)
instead of a task. Reach for the sync twins only in one-shot scripts and build
tools, where there's no concurrency to starve; in a server, prefer the async
default so one slow file doesn't stall everything else.

## Reading

`read-text` reads a whole file as a string, `read-bytes` as raw bytes, and
`read-dir` lists a directory's entries. Each can fail — the file might not exist,
the directory might not be readable — so each returns a `result` (sync) or a
failing `task` (async), with the operating system's message on the `err` side:

```pluma
use std/sys/fs

def load-config :: fun string -> task string string = fun path {
	try text = fs.read-text path
	task.return text
}
```

In a script, the sync twin reads the same way without the `task`:

```pluma
use std/sys/fs

when fs.read-text-sync "config.txt" is ok text {
	# use text
} is err message {
	# the file was missing or unreadable
}
```

## Writing

`write-text` replaces a file's contents (creating it if needed), and `append-text`
adds to the end. `write-bytes` and `append-bytes` do the same with raw bytes:

```pluma
use std/sys/fs

try _ = fs.write-text "log.txt" "started\n"
try _ = fs.append-text "log.txt" "ready\n"
```

A write returns nothing on success, so the `try _ =` just awaits it and forwards
any failure.

## Asking about a path

The plain queries — `exists`, `is-file`, `is-dir` — answer with a `bool`, not a
`result`, because "no, it isn't there" is an ordinary answer rather than an error:

```pluma
use std/sys/fs

fs.exists-sync "config.txt"   # => true or false
```

For more detail, `fs.stat` returns a `file-info` record — `{size, kind,
modified}` — where `kind` says whether the path is a file, a directory, or a
symlink, and `modified` is an [instant](/docs/stdlib/time) you can format or
compare.

## Managing files and directories

The rest of the module moves things around: `make-dir` creates a directory,
`remove` deletes a single file or empty directory, `remove-all` deletes a
directory and everything in it, and `rename` and `copy` do what they say. All are
async with `-sync` twins, and all return a `result`/`task` since any of them can
fail.

```pluma
use std/sys/fs

try _ = fs.make-dir "build"
try _ = fs.copy "template.html" "build/index.html"
```

## Building paths

To assemble and pick apart paths without hand-splicing strings and slashes, use
[`std/path`](/std/path): `path.join` combines segments with the right separator,
and `path.filename`, `path.parent`, `path.extension`, and `path.stem` pull a path
apart.

```pluma
use std/path

path.join "build" "index.html"   # => "build/index.html"
path.extension "notes.md"        # => some "md"
```

## See also

- **[Concurrency](/docs/reference/concurrency)** — the `task` every async op
  returns, and how `try` awaits it.
- **[Bytes](/docs/reference/bytes)** — what `read-bytes` and `write-bytes` work
  with.
