# Command-line script

Pluma works as a scripting language: a single file with a `main`, run on the
spot. This one greets every name you pass on the command line.

```pluma
use std/list
use std/sys/process

# Run it with:  pluma run greet.pa -- Ada Grace
def main = fun {
	let names = process.args ()
	if list.is-empty names {
		print "usage: greet <name>..."
	} else {
		list.each names fun name {
			print "hello, $(name)!"
		}
	}
}
```

Run it with `pluma run greet.pa -- Ada Grace`. `process.args` hands you the
arguments as a `list string`, and the source compiles to WebAssembly and runs
with no startup wait.
