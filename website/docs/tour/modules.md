# Modules

Every file is a module. Bring another one into scope with `use`, then reach its
contents through its name:

```pluma
use std/list

def main = fun {
	let xs = list.map [1, 2, 3] (fun n { n * 2 })
	print (to-string (list.sum xs))   # 12
}
```

Import paths use `/` to separate segments — `use std/list`, `use std/sys/fs` —
not dots. The last segment is the name you reach the module through, so `use
std/list` lets you write `list.map`, `list.sum`, and so on.

A handful of names are so common that Pluma imports them for you: `option`,
`result`, and `ref` need no `use`. Everything else you ask for explicitly,
including modules like `std/task` — naming an import is how a reader of your file
knows where a name comes from.

## Public and private

Within a file, definitions are private by default — visible only there. Mark one
`public` to let other modules use it:

```pluma
public def add = fun x y { x + y }   # other files can use this
def helper = fun n { n + n }         # private to this file
```

Keeping things private by default means a module's surface is exactly what it
declares `public`, nothing leaks by accident, and you can rework the private
parts freely. An enum has a third option, `opaque`, which exports the type but
hides its variants — callers can hold one and pass it around, but only the module
can build or take one apart. That's how `std/error` keeps its internal frame
structure to itself while still handing you an `error` to carry.

## Your own modules

There's nothing special about standard-library modules — your own files work the
same way. A file `geometry.pa` is the module `geometry`, and from a neighbouring
file you write `use geometry` and call `geometry.area`. Splitting a program
across files is just this: `public` what the rest of the app needs, `use` it
where you need it.

That's the whole language. From here, the [reference](/docs/reference) pages go
deeper on individual topics, and the [guides](/docs/guides/cli) walk through
building a real command-line tool, web server, and fullstack app.
