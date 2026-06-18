# Get started

From the playground to the compiler on your machine, in a few minutes.

## Try it first

Nothing to install to get going: the [playground](/playground) runs the real
compiler and your program right in the browser. Write a little Pluma, press Run,
and watch it work.

When you want it on your own machine, install the compiler below.

## Install

Pluma builds from source with a stable `Rust` toolchain, the only
prerequisite. Clone the repo and build the `pluma` binary:

```
git clone https://github.com/reidmit/pluma
cd pluma
cargo build --release --bin pluma
```

The compiler is now at `target/release/pluma`. To put it on your `PATH` instead,
install it:

```
cargo install --path cli
```

Editor support ships in the repo too: a language server (`pluma
language-server`) with VS Code and Zed integrations for inline diagnostics,
hovers, and formatting.

## Your first program

A Pluma program is a list of definitions; running it runs the one named `main`.
Put this in a file called `main.pa`:

```pluma
def main = fun {
	print "hello, world"
}
```

Then run it: the source compiles to WebAssembly and executes on the spot:

```
pluma run main.pa
# hello, world
```

That's the whole loop. From here, the [reference](/docs/reference) walks through
the language one idea at a time, and the [playground](/playground) runs this
exact compiler in your browser.

## The toolbelt

One binary covers the workflow. Each command takes a `.pa` file or a directory
containing a `main.pa`.

| Command | What it does |
| --- | --- |
| `pluma run main.pa` | Compile to WasmGC and run it under V8 |
| `pluma build main.pa` | Produce a deployable `.wasm` artifact |
| `pluma dev main.pa` | Watch sources and re-run on every save |
| `pluma test` | Discover and run every `*.test.pa` suite |
| `pluma format .` | Canonicalize formatting in place |
| `pluma lint .` | Report stylistic and correctness smells |

Tests are a library, not syntax: a `*.test.pa` file exports a list of cases
built from `std/test`, and `pluma test` runs them under V8, the same engine
your built artifact deploys to.

## Going fullstack

Point `pluma` at a directory holding a `server.pa` and a `client.pa` and it
builds a fullstack app: one language from the database to the DOM. A `remote
def` becomes a typed RPC: the compiler writes the server route and the client
stub, so the two agree by construction.

```
app/
	server.pa   # the http handler + remote defs
	client.pa   # boots the browser, hydrates the server's HTML
```

Develop it with live-reload, then build the deployable bundle:

```
pluma dev app/      # live-reload dev server
pluma build app/    # server .wasm + browser bundle
```

This very website is exactly that: a fullstack Pluma app, server-rendered for an
instant first paint, then hydrated in the browser.

Next: [read the reference](/docs/reference), [open the
playground](/playground), or [browse the
examples](https://github.com/reidmit/pluma/tree/main/examples).
