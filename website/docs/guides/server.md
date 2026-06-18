# Web server

Stand up an HTTP server with typed routes. `router.handler` turns a list of
routes into a request handler, and `http.serve` runs it.

```pluma
use std/router
use std/task
use std/sys/http

def app :: fun http.request -> task http.response = router.handler [
	router.get "/" fun _req {
		task.return (http.text 200 "hello from Pluma")
	},
	router.get "/health" fun _req {
		task.return (http.text 200 "ok")
	},
]

def main = fun {
	http.serve "127.0.0.1:8080" app
}
```

Run `pluma run server.pa` and open `http://127.0.0.1:8080`. Each route returns a
`task http.response`, so the server handles many requests at once, and failures
come back as values you handle, not surprises that escape.
