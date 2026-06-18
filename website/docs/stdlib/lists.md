# Working with lists

The [tour](/docs/tour/collections) introduced lists — ordered collections where
every element has the same type, written `[1, 2, 3]`. This page is the working
tour of `std/list`, the module that does the real work: transforming, searching,
slicing, and sorting.

Lists are immutable. Everything here that "adds," "removes," or "sorts" hands you
back a brand-new list and leaves the original alone — with a small set of
deliberate exceptions covered at the end. Many of these functions take a small
function you write inline: a *predicate* that answers true or false about one
element (`fun n { n > 0 }`), or a *transform* that turns it into something else
(`fun n { n * 2 }`).

## The three you'll use most

`map` transforms every element, `filter` keeps the ones that pass a test, and
`fold` boils the whole list down to a single value:

```pluma
use std/list

list.map [1, 2, 3] (fun n { n * 2 })             # => [2, 4, 6]
list.filter [1, 2, 3, 4] (fun n { n % 2 == 0 })  # => [2, 4]
list.fold [1, 2, 3, 4] 0 (fun acc n { acc + n }) # => 10
```

`fold` is the general one. You give it the list, a starting value, and a function
`f acc element`; it walks front to back, calling `f` with the running total and
the next element, and whatever `f` returns becomes the new total. Most other
list operations could be written as a `fold` — `map` and `filter` are just the
common shapes worth naming. When you want to run a side effect for each element
rather than build a value, `each` does that and returns nothing.

## Reaching elements, safely

Asking for an element that might not exist returns an
[`option`](/docs/reference/errors) instead of risking a crash. `head` gives the
first, `last` the final one, and `find` the first that passes a test:

```pluma
use std/list

list.head [1, 2, 3]                      # => some 1
list.head []                             # => none
list.find [1, 2, 3, 4] (fun n { n > 2 }) # => some 3
```

`get xs i` reads the element at a position directly — but unlike these, it's
*partial*: an index outside the list stops the program rather than returning an
option. Use `get` when you've already established the index is valid (a loop
counter, a `find-index` result); reach for `head`/`find` when emptiness is
genuinely possible.

## Slicing and reshaping

```pluma
use std/list

list.take [1, 2, 3, 4, 5] 3              # => [1, 2, 3]
list.drop [1, 2, 3, 4, 5] 2              # => [3, 4, 5]
list.reverse [1, 2, 3]                   # => [3, 2, 1]
list.concat [1, 2] [3, 4]                # => [1, 2, 3, 4]
list.unique [1, 2, 1, 3, 2]              # => [1, 2, 3]
list.flatten [[1, 2], [], [3, 4]]        # => [1, 2, 3, 4]
list.flat-map [1, 2, 3] (fun n { [n, n * 10] })  # => [1, 10, 2, 20, 3, 30]
```

`flat-map` is `map` followed by `flatten` in one step — transform each element
into a list, then concatenate them — which is the natural tool when each input
produces zero, one, or several outputs. Nearby in the module: `take-while` and
`drop-while` (cut at the first element that fails a test), `chunk` (split into
fixed-size groups), `partition` (split into the passes and the fails), and
`intersperse` (put a separator between elements).

## Searching and testing

These answer a question about the list rather than rebuilding it:

```pluma
use std/list

list.contains [1, 2, 3] 2                # => true
list.any [1, 2, 3] (fun n { n > 2 })     # => true
list.all [2, 4, 6] (fun n { n % 2 == 0 })# => true
list.count [1, 2, 3, 4] (fun n { n % 2 == 0 })  # => 2
list.find-index [10, 20, 30] (fun n { n > 15 }) # => some 1
```

## Sorting and aggregates

`sort` orders a list given a comparison; pass `ord.compare` to sort by the
natural order of any [comparable](/docs/tour/traits) type. `sort-by` is the
common convenience — sort by a key extracted from each element:

```pluma
use std/list

list.sort [3, 1, 4, 1, 5] ord.compare            # => [1, 1, 3, 4, 5]
list.sort-by ["bbb", "a", "cc"] string.length    # => ["a", "cc", "bbb"]
```

The aggregates `min`, `max`, `sum`, and `product` return an `option`, since an
empty list has no smallest element and no meaningful total:

```pluma
list.max [3, 1, 4, 1, 5]   # => some 5
list.sum [1, 2, 3]         # => some 6
list.max []                # => none
```

## Pairing up: zip and enumerate

`zip` pairs two lists element by element (stopping at the shorter), and
`enumerate` pairs each element with its index — handy when a transform needs to
know where it is:

```pluma
use std/list

list.zip [1, 2, 3] ["a", "b"]      # => [(1, "a"), (2, "b")]
list.enumerate ["a", "b", "c"]     # => [(0, "a"), (1, "b"), (2, "c")]
```

## Building, and the one mutable escape hatch

`build n f` tabulates a list from a function of the index, and `range` produces a
run of integers:

```pluma
use std/list

list.build 4 (fun i { i * i })   # => [0, 1, 4, 9]
list.range 0 5                   # => [0, 1, 2, 3, 4]
```

Most of the time you grow a list by transforming another one. But when you're
accumulating in a loop and don't know the final size, `push` appends to the end
*in place* in amortized constant time — and `pop` removes the last element,
making a list a fast stack:

```pluma
use std/list

let acc = []
list.push acc 1
list.push acc 2
acc            # => [1, 2]
```

::: aside .callout
**`push`, `pop`, and `set` mutate the list in place** — the one exception to
lists being immutable. Because a list's storage is shared, the change shows up
through *every* name pointing at that same list. So use them only on a list you
just built and still solely own (the fresh-accumulator pattern above). When in
doubt, build a fresh list with `build`, `map`, or a spread instead — that's the
norm, and it can never surprise another part of your program.
:::

Reaching for `push` in a loop matters for speed, too: the tempting `[...acc, x]`
copies the whole list on every step, which is fine for a handful of elements but
quietly O(n²) over a large one. `push` avoids that copy.

## See also

- **[Lists, tuples, and records](/docs/tour/collections)** — list literals and
  the spread syntax.
- **[Dictionaries and sets](/docs/stdlib/dict-set)** — when you look up by key or
  test membership instead of walking in order.
