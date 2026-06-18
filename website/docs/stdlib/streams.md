# Streams

A `stream a` is a sequence of values produced over time and consumed one at a
time: a file read line by line, a series of events sent from a server, anything
too large or too open-ended to hold in a list all at once.

What sets it apart from a [list](/docs/stdlib/lists) is *when* the values exist. A
list is all there, in memory, now. A stream is **pull-based**: the consumer asks
for the next value, the producer computes just that one, and nothing runs ahead of
demand. A slow consumer simply pulls slowly, and the producer waits, so a stream
never races ahead and floods memory. That automatic restraint is called
backpressure, and you get it for free.

Each value (and the end-of-stream signal) rides a [`task`](/docs/reference/concurrency),
so producing one can do real asynchronous work (read from a socket, query a
database), and a producer failure surfaces as a task failure on whoever is
pulling.

## Building a stream

The general builder is `from-seed`: you give it a starting state and a step
function that, from the current state, produces the next value and the next state
(or `none` to end). Here's an endless stream of counting numbers:

```pluma
use std/stream
use std/task

def counting :: fun nothing -> stream int = fun {
	stream.from-seed 1 (fun n { task.return (some (n, n + 1)) })
}
```

It never ends, and that's fine, because nothing is computed until it's pulled.
When a stream owns something that must be cleaned up (an open file, a connection),
`from-resource` builds it with a release step that's guaranteed to run when the
stream is done.

## Transforming, lazily

You reshape a stream with the familiar operations (`map`, `filter`, `take`,
`take-while`), and they're all *lazy*: each one describes work to do as values
flow through, but computes nothing on its own. The pipeline only moves when a
consumer pulls from the end of it.

`take` is how you bound a stream, including an endless one:

```pluma
stream.take (stream.map (counting ()) (fun n { n * 2 })) 5
# the stream 2, 4, 6, 8, 10: five values, then it ends
```

## Consuming a stream

Nothing actually happens until a consumer drives the stream to its end. `for-each`
runs a function for every value, `fold` boils the whole stream down to one value,
and `drain` just runs it for its effects:

```pluma
use std/stream
use std/task

try _ = stream.for-each (stream.take (counting ()) 5) (fun v {
	print (to-string v)
	task.return ()
})
# prints 1 2 3 4 5

try total = stream.fold (stream.take (counting ()) 4) 0 (fun acc x {
	task.return (acc + x)
})
# total => 10
```

The consumers also handle cleanup: a resource-owning stream releases its resource
**exactly once**, whether it finishes normally, fails, or is cut short. That last
case matters: *early exit is bounding the stream with `take`, not breaking out of
a loop*. The bound ends the upstream and runs its release for you, so you never
leak a half-read file. (For hand-rolled looping, `stream.next` pulls one value at a
time, but then the cleanup is yours to run.)

## One consumer

A stream is single-consumer: pulling a value consumes it, so two readers can't
share one stream. When you need to feed the same data to several places, collect
it or build a stream per consumer.

## Where streams show up

The clearest use is server-to-client streaming: a [server](/docs/guides/server)
can answer a request with a stream of events instead of one fixed body, writing
each as it's ready: the foundation for live updates and progress feeds. The same
shape fits reading a large file or paging through a big query without loading it
all at once.

## See also

- **[Concurrency](/docs/reference/concurrency)**: the `task` each stream value
  rides on.
- **[Working with lists](/docs/stdlib/lists)**: when the whole sequence fits in
  memory and you want it all at once.
