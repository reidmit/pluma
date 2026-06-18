# Queues

A queue is a first-in, first-out (FIFO) collection: you add values to the back and
take them from the front, so they come out in the order they went in, like a line
at a counter. Reach for one when arrival order matters: work waiting to be
processed, the frontier of a breadth-first search, events to replay in sequence.

`std/queue` rounds out the collection family alongside
[lists, dicts, and sets](/docs/stdlib/dict-set). What makes it different is the
access pattern (strictly front-and-back) and one other thing worth stating up
front: unlike those, a queue is **mutable**.

## Adding and removing

`enqueue` adds a value to the back, and `dequeue` removes and returns the one at
the front. Because the queue might be empty, `dequeue` returns an
[`option`](/docs/reference/errors): `none` when there's nothing left:

```pluma
use std/queue

let q = queue.empty ()
queue.enqueue q 1
queue.enqueue q 2
queue.dequeue q   # => some 1   (the first one in)
queue.dequeue q   # => some 2
queue.dequeue q   # => none     (now empty)
```

`peek` looks at the front element without removing it, and `queue.from-list`
builds a queue from a list (front-to-back, so the first list element dequeues
first). `size` and `is-empty` answer the obvious questions, and `to-list` takes a
snapshot of the contents without draining them.

```pluma
queue.from-list [1, 2, 3]                 # dequeues 1, then 2, then 3
queue.size (queue.from-list [1, 2, 3])    # => 3
```

## A queue mutates in place

This is the part to keep in mind. `enqueue` and `dequeue` change the queue itself
rather than handing back a new one, the same way [`list.push`](/docs/stdlib/lists)
grows a list in place. That's what makes both operations fast (amortized constant
time, so draining a large queue stays cheap), but it means a queue follows the
same aliasing rule: the change shows up through *every* name pointing at the same
queue.

So treat a queue as owned by the code filling and draining it: pass it around
within that, but don't hand the same queue to two parts of a program expecting
independent copies. When you need a stable, shareable view of the contents, take a
`to-list` snapshot instead.

## A worked example

The natural shape is a drain loop: pull from the front until it's empty.

```pluma
use std/queue

def drain :: fun (queue int) -> nothing = fun q {
	when queue.dequeue q is some x {
		print (to-string x)
		drain q          # keep going until dequeue returns none
	} else {
		()
	}
}
```

Each `dequeue` removes one element and hands it back, so the loop visits every
value in arrival order and stops on its own when the queue runs dry.

## See also

- **[Dictionaries and sets](/docs/stdlib/dict-set)**: the other collections, and
  when to pick which.
- **[Working with lists](/docs/stdlib/lists)**: including `push`/`pop`, the
  list's own in-place stack operations.
