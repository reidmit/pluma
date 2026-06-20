# Dictionaries and sets

The [tour](/docs/tour/collections) covered lists, tuples, and records. Two more
collections from the standard library round out the everyday toolkit: a
*dictionary*, which looks values up by key, and a *set*, which remembers whether
a value is present. Both live behind a `use` (`std/dict` and `std/set`), and
both are immutable values, like everything else in Pluma.

## Dictionaries

A dictionary maps keys to values. `dict k v` reads "a dict from `k` to `v`," so
`dict string int` maps strings to whole numbers. Unlike a list, it finds a value
by its key rather than its position, and each key appears at most once.

You start from an empty dict and build it up. Each `insert` returns a *new* dict
rather than changing the old one, so you rebind the name as you go:

```pluma
use std/dict

let scores = dict.empty ()
let scores = dict.insert scores "ada" 10
let scores = dict.insert scores "alan" 8
let scores = dict.insert scores "ada" 11   # replaces ada's 10
```

### Looking things up

A lookup might find nothing, so `dict.lookup` returns an
[`option`](/docs/reference/errors): `some value` when the key is present, `none`
when it isn't. That pairs naturally with `??` to supply a default:

```pluma
(dict.lookup scores "ada") ?? 0    # => 11
(dict.lookup scores "grace") ?? 0  # => 0   (absent → the default)
```

`dict.contains-key` answers the yes/no question directly, and `dict.size` counts
the entries.

### Updating in place of a key

When the new value depends on the old one (counting occurrences, say),
`dict.update` hands you the current value (as an `option`, since the key may be
new) and stores whatever you return:

```pluma
let counts = dict.update scores "ada" (fun cur { (cur ?? 0) + 1 })
(dict.lookup counts "ada") ?? 0   # => 12
```

### Walking the entries

`dict.keys`, `dict.values`, and `dict.entries` give you the contents as lists
(`entries` as a list of `(key, value)` tuples), and `dict.map`, `dict.filter`,
and `dict.fold` transform a dict without unpacking it by hand. One caveat worth
knowing: the order is unspecified. A dict tracks *which* keys it holds, not any
sequence, so don't rely on iteration coming out sorted.

## Sets

A set is an unordered collection of distinct values. `set a` is "a set of `a`."
It remembers *whether* a value is present, not how many times or in what order:
adding a value that's already there changes nothing, so a set never holds
duplicates. Reach for one when you care about membership: "have I seen this id?",
"which tags are in use?"

Building one from a list is the quickest way to drop duplicates:

```pluma
use std/set

let tags = set.from-list ["red", "blue", "red", "green", "blue"]
set.size tags            # => 3
set.contains tags "blue" # => true
```

### Set algebra

The real convenience of a set is combining two of them. `union` keeps everything
in either, `intersection` keeps only what's in both, and `difference` keeps
what's in the first but not the second:

```pluma
use std/set

let a = set.from-list [1, 2, 3]
let b = set.from-list [2, 3, 4]

set.to-list (set.union a b)         # the values 1, 2, 3, 4 (order unspecified)
set.to-list (set.intersection a b)  # the values 2, 3
set.to-list (set.difference a b)    # the value 1
```

There are membership comparisons to match (`subset-of`, `superset-of`,
`disjoint`, and `equals`), each answering a yes/no question about how two sets
relate.

## Keys must know how to hash

Both collections find an element by hashing it, so a key (in a dict) or a member
(in a set) has to be a type that knows how to `hash` itself. The built-in types
(`int`, `float`, `string`, `bytes`, `bool`) all do, so they work with no setup.
Your own enum can join them by implementing the `hash`
[trait](/docs/tour/traits), the same way a type opts into `to-string` or
comparison. Until it does, the compiler won't let you use it as a key: that's
the `where (hash k)` you'll see in these functions' signatures.

## Which to reach for

- A **list**: when order and duplicates matter, and you walk it front to back.
- A **dict**: when you look values up by a key.
- A **set**: when you only care whether something is a member, and want
  duplicates collapsed.

All three are immutable, so passing one to a function never lets the function
change the copy you kept.
