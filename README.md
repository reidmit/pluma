# pluma

a small statically-typed functional language. source files use the `.pa` extension; the CLI binary is `pluma`.

```
def main = fun {
	print "hello, world"
}
```

## taste

```
enum tree {
	empty
	node int tree tree
}

def sum = fun t {
	when t is empty { 0 }
	is node n l r { n + sum l + sum r }
}

def main = fun {
	let t = node 1 (node 2 empty empty) (node 3 empty empty)
	print (sum t)
}
```

typeclasses with dictionary-passing dispatch:

```
trait showable a {
	show :: a -> string
}

implement showable int {
	def show = fun x { to-string x }
}

implement showable bool {
	def show = fun b { when b is true { "yes" } else { "no" } }
}

def main = fun {
	print (show 42)
	print (show true)
}
```

generic enums with parametric instances:

```
enum option a {
	some a
	none
}

implement ord (option a) where (ord a) {
	def compare = fun x y {
		when x is some xv {
			when y is some yv { compare xv yv } is none { gt }
		}
		is none {
			when y is some _v { lt } is none { eq }
		}
	}
}
```

see [`REFERENCE.md`](REFERENCE.md) for the full language.
