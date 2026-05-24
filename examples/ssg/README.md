# ssg — a static site generator in Pluma

A small but real static site generator, written entirely in Pluma. It
reads Markdown files (with optional front-matter), converts them to
HTML, and wraps each in a styled page template — plus an index linking
them all.

It exists to exercise the language on something bigger than a fixture:
recursive-descent parsing, algebraic data types, pattern matching,
string/byte manipulation, maps, and real file I/O.

## Run it

```
pluma run examples/ssg
```

That builds `examples/ssg/content/*.md` into `examples/ssg/public/`.
You can point it elsewhere by passing arguments:

```
pluma run examples/ssg <content-dir> <output-dir>
```

Open `examples/ssg/public/index.html` in a browser to see the result.

## Layout

| File          | Role                                                            |
|---------------|-----------------------------------------------------------------|
| `markdown.pa` | Markdown → HTML: an AST of `enum`s, a block parser, an inline scanner, and a renderer. |
| `page.pa`     | Front-matter (`key: value`) parsing and the HTML page template. |
| `main.pa`     | The driver: walk the content dir, build each page, write output, build an index. |
| `content/`    | Sample Markdown sources.                                        |
| `public/`     | Generated output (created on build).                            |

## Supported Markdown

**Blocks:** ATX headings (`#`–`######`), fenced code blocks, blockquotes,
bullet and ordered lists, thematic breaks (`---`), paragraphs.

**Inline:** `**bold**`, `*italic*`, `` `code` ``, and `[links](url)`.
Text is HTML-escaped; code spans are kept literal.

## How it works

The pipeline is three passes:

1. **Block parse** (`markdown.pa`) — split the body into lines, then
   recursively group them into `block` nodes. Multi-line constructs
   (code fences, lists, blockquotes, paragraphs) each consume a run of
   lines and hand back the remainder.
2. **Inline scan** — within a block's text, a byte-level scanner walks
   the line emitting `inline` spans. Because every Markdown marker is
   ASCII, scanning bytes is UTF-8-safe.
3. **Render** — a pair of `when` matches turn the `block`/`inline` AST
   into HTML strings, which `page.pa` wraps in a full document.
