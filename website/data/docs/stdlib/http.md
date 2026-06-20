# Making HTTP requests

`std/sys/http` is both sides of HTTP: the server (covered in the
[web server guide](/docs/guides/server)) and a client for *making* requests. This
page is about the client: `http.fetch`, for calling another service from your
own.

## A single request

`http.fetch` makes one HTTP/1.1 request and returns the reply. It takes four
things (the URL, the method, a dictionary of headers, and the body as bytes),
and hands back a [`task`](/docs/reference/concurrency), since a network round-trip
is asynchronous:

```pluma
use std/dict
use std/string
use std/sys/http

def ping :: fun string -> task http.response string = fun url {
	try resp = http.fetch url http.method.get (dict.empty ()) (string.to-bytes "")
	task.return resp
}
```

The method is a value from the `http.method` enum: `http.method.get`,
`http.method.post`, `http.method.put`, `http.method.delete`, `http.method.patch`.
A request with no body or headers passes an empty bytes value and an empty dict.

## Reading the reply

A `response` is a record: `status` is the numeric code, `headers` is a
dictionary, and `body` is raw **bytes**. Decode the body to text with
`bytes.to-string`, which returns an [`option`](/docs/reference/errors) in case the
bytes aren't valid text:

```pluma
use std/bytes

try resp = http.fetch url http.method.get (dict.empty ()) (string.to-bytes "")
let text = bytes.to-string resp.body ?? "<bad response>"
# resp.status is the code; text is the body
```

Bytes in and bytes out is deliberate: it means HTTP carries anything, not just
text. To send and receive [JSON](/docs/stdlib/json), pair `fetch` with the JSON
module: `string.to-bytes (json.stringify payload)` on the way out, and
`json.parse text` on the reply.

## When it fails

The request's `task` *fails*, with a message, if the host can't be reached or
the exchange breaks down. As with any task, `try` propagates that failure; when
you'd rather inspect it, `task.attempt` turns the task into a
[`result`](/docs/reference/errors):

```pluma
use std/task

try outcome = task.attempt (http.fetch url http.method.get (dict.empty ()) (string.to-bytes ""))
# outcome is err "..." on a connection failure, rather than crashing
```

One limit to know: the client speaks plain HTTP over TCP, with **no TLS**. An
`https://` URL fails rather than silently downgrading. It's built for talking to
services on your own network or machine, not the public web.

## Prefer a remote def between your own server and client

`http.fetch` is the low-level escape hatch: reach for it to call a third-party
service. When the two ends are *your own* Pluma server and browser, you don't
write `fetch` calls by hand at all: a [`remote def`](/docs/deep-dives/rpc) becomes
a typed call where the compiler generates the request and response handling, so
the two sides can't disagree about the shape of the data. Use `fetch` for the
outside world; use a `remote def` for your own app.

## See also

- **[Web server](/docs/guides/server)**: the serving half of `std/sys/http`.
- **[How RPC works](/docs/deep-dives/rpc)**: typed server calls with `remote
  def`, built on this transport.
- **[JSON](/docs/stdlib/json)**: encoding and decoding request and response
  bodies.
