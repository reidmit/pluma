# jq-lite

A tiny [`jq`](https://jqlang.org)-style JSON query tool, written in Pluma.
It reads a JSON document, walks it with a path expression, and
pretty-prints each value the path selects.

```
pluma run examples/jq <path> [file.json]
```

If the file is omitted, the document is read from stdin.

## Path expressions

| Syntax        | Meaning                                          |
| ------------- | ------------------------------------------------ |
| `.`           | identity — the whole document                    |
| `.field`      | object field                                     |
| `.a.b.c`      | chained fields                                   |
| `.a[0]`       | array index (negative counts from the end)       |
| `.a["key"]`   | quoted field, for keys with dots or spaces       |
| `.a[]`        | iterate: every array element / object value      |

Like `jq`, evaluation is **stream-based**: each step maps the current
stream of values to a new one. `[]` is the only step that grows the
stream; a missing field or out-of-range index drops that value
silently, so a path matching nothing simply prints nothing.

## Examples

```
$ pluma run examples/jq '.team' examples/jq/team.json
"platform"

$ pluma run examples/jq '.users[0].name' examples/jq/team.json
"Ada"

$ pluma run examples/jq '.users[].name' examples/jq/team.json
"Ada"
"Linus"
"Grace"

$ pluma run examples/jq '.users[-1].roles[0]' examples/jq/team.json
"admin"

$ echo '{"a":[1,2,3]}' | pluma run examples/jq '.a[-1]'
3

$ echo '{"a.b": 42}' | pluma run examples/jq '.["a.b"]'
42
```

## Layout

- `query.pa` — the `step` enum, the path parser (`parse-path`), and the
  stream evaluator (`eval-path`). The parser walks the path string with
  `string.char-at`/`slice`; the evaluator threads a `list value` through
  each step with `list.fold`, navigating via the `std/json` accessors
  and `dict.values`.
- `main.pa` — CLI glue: read args/stdin, `json.parse`, run the query,
  `json.stringify-pretty` each result. Parse, path, and read errors are
  reported distinctly.
- `team.json` — sample document.
