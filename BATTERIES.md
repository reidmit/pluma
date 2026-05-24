# BATTERIES.md

Roadmap for Pluma's batteries-included standard library — the full
set of modules we want bundled in the language binary so that real
programs (scripts, web services, data-munging, tests, dev tools) can
be written without reaching for third-party packages.

This is the *master list*, not a commitment to ship everything at
once. We prune and prioritize against it.

The stdlib-authoring mechanism this all relies on — `.pa` source files
with `built-in "tag"` expressions backed by the `Builtin` enum — has
shipped; the modules already under `compiler/src/stdlib/*.pa` are the
pattern to copy. `try` chaining (relevant to the fallible APIs below)
has also shipped for `option`/`result`. The one remaining companion
plan is [ASYNC.md](ASYNC.md) — `task a` and structured concurrency, not
yet built; several modules below depend on it.

**Shipped so far:** `core.list`, `core.string`, `core.math`,
`core.bytes`, `core.regex`, `core.io`, `core.ref`, `core.option`,
`core.result`, `core.map`, `core.json`, `core.base64`, `core.hex`,
`core.uuid`, `core.random`, `core.assert`, plus the `test "..." { }`
form + `pluma test` runner and the `pluma.pa`/`core.package` project
system. The big still-missing clusters: scripting (`fs`/`process`/`env`/
`glob`/`term`), data formats (`csv`/`toml`/`url`), `sqlite`, `html`/`xml`,
`time`, and everything under Tier 3 (waits for async).

Read top-to-bottom for the catalog; jump to "Sequencing" to decide
what to build next.

## Why batteries-included

Concrete capabilities this unlocks:

- **Pluma is enough on its own.** No `pip install` or `npm i` step
  for the 80% case. Single binary, ship it. Matches the project's
  "language, not framework" framing — except for foundational
  utilities, which a *language* without is dead in the water.
- **One canonical option per domain.** Every JSON parser, every
  HTTP client, every test framework is the official one. No
  ecosystem fragmentation when there's no ecosystem yet.
- **Doc/LSP discoverability.** Every stdlib def carries a doc comment
  visible in hover and "go to definition" lands on readable `.pa`
  source.
- **Fast iteration on the language itself.** When we have JSON,
  HTTP, SQLite, tests, and a logger in the box, we can build real
  apps in Pluma — which surfaces real language pain points faster
  than synthetic toys do.

Things we deliberately do NOT include:

- **A package manager.** External libraries are a separate problem.
  Don't conflate "batteries-included stdlib" with "ecosystem."
- **Multi-implementation choice within a domain.** One JSON parser.
  One HTTP client. One test framework. If users want alternatives,
  that's a userland problem later.
- **GUI / view / templating frameworks.** Out of scope for the
  language stdlib.
- **Bindings to operating-system-specific APIs.** Cross-platform
  surface only. (Process spawning is fine; ioctls are not.)
- **Heavy ML / numerics / graphics.** No tensor library, no image
  codecs, no audio. Userland.

## Architectural pieces

These are shared infrastructure that several modules below depend on.
Worth nailing down once and reusing.

### Vendoring Rust crates

The default implementation pattern: established Rust crate → thin
Rust wrapper exposing operations as `Builtin` enum variants → `.pa`
surface using `built-in "tag"` (see the existing
`compiler/src/stdlib/*.pa` modules).

Crate-selection guidelines:

- Pick crates with strong maintenance, MIT/Apache licenses, minimal
  transitive deps.
- Pick crates where a dominant choice clearly exists — don't
  bikeshed (`serde_json`, `rusqlite`, `rustls`, `chrono`, `url`,
  `uuid`, `regex`, `flate2`, etc.).
- For each crate, prefer features that produce *pure-Rust* builds
  (e.g., `rusqlite` with `bundled` feature, `rustls` instead of
  OpenSSL bindings) so we keep the single-binary distribution story
  intact.
- Pay attention to binary-size cost. We're not chasing minimalism,
  but bloat per stdlib module should be tracked.

When *not* to vendor:

- The function is a one-liner over existing builtins (`option.is-some`,
  `list.is-empty`).
- The crate brings disproportionate transitive deps for a small
  feature.
- The behavior is specific enough to Pluma's representations that
  writing it in-tree is cheaper than mapping a generic API.

### Opaque handles

Several modules below need values that aren't decomposable Pluma
data — SQLite connections, HTML DOM trees, HTTP response streams,
random-state seeds, etc. The VM needs a representation for these.

Two options for the design:

**Option A — single `Value::Opaque(Rc<dyn Any>)` variant.** One
catch-all variant; each module defines an internal `struct` and
casts in/out via `Any::downcast`. Simplest VM change. Type-level
representation needs a corresponding `Type::Opaque(name)` so the
analyzer can distinguish a `sqlite.connection` from an
`html.document`.

**Option B — per-domain variants** like `Value::SqliteConn`,
`Value::HtmlNode`. Better tracing/debugging (you see the actual
type in `{:#?}` output), but the VM grows a variant per module.

Prior art in this codebase: `Type::Regex` + `RegexData` already
treats compiled regex as a non-decomposable native value (see
`codegen/src/emit.rs`). That's effectively Option B at very small
scale.

**Recommendation:** start with Option A. It scales to any number of
modules without VM changes. Add per-domain variants later only if
profiling shows downcast overhead matters. Spell this out in a small
follow-up design note before the first heavy module lands (probably
JSON-then-SQLite).

Pluma-side, opaque types are nominal: declared in the stdlib `.pa`
file with no constructors and no field access. Operations on them
are stdlib functions that pattern-match internally via the builtin.
This is the same shape as `regex` today.

### Namespacing

Everything lives under `core.*` for now. Two flat levels: `core.json`,
`core.sqlite`, `core.http.client`, `core.http.server`. The current
codebase already uses `core.list`, `core.string`, etc., so this is
consistent.

Open question: at some point we may want a tier distinction — e.g.,
`core.*` for true essentials (list, string, math, option, result)
and `std.*` for the broader batteries (sqlite, http, html). Not
worth deciding now; punt until the surface area starts hurting
discoverability.

## The four called-out modules

### `core.json`

Recommended crate: **`serde_json`**.

Surface sketch:

```pluma
enum json.value {
    null
    bool   bool
    int    int        # parsed as int when no decimal point / exponent
    float  float      # parsed as float otherwise
    string string
    array  (list json.value)
    object (map string json.value)
}

# Total — never throws.
def parse :: fun string -> result json.value json.error =
    built-in "json-parse"

# Pretty-print with 2-space indent.
def stringify        :: fun json.value -> string = built-in "json-stringify"
def stringify-pretty :: fun json.value -> string = built-in "json-stringify-pretty"

# Walker helpers — return option/result rather than panicking.
def get-field :: fun json.value string -> option json.value
def get-int   :: fun json.value -> result int json.error
def get-array :: fun json.value -> result (list json.value) json.error
...
```

Design notes:

- **Split `int`/`float`** in `json.value` rather than collapsing
  everything to `float`. Pluma already separates the types; losing
  precision on big ints in a JSON parser would be a footgun.
- **No automatic decode-to-user-type.** That requires reflection or
  a derive mechanism we don't have. Users hand-write a decoder by
  walking `json.value`. A follow-up doc can design a derive story
  when we're ready.
- **`json.error`** carries `line`, `col`, `message`. Total parse —
  never throws or panics.
- The walker helpers (`get-field`, `get-int`, etc.) are pure-Pluma
  one-liners over the `value` ADT. No builtin needed.

### `core.html` and `core.xml`

Two libraries, not one.

Recommended crates:
- HTML: **`html5ever`** for full browser-grade parsing, or
  **`tl`**/**`scraper`** for lighter. Decide based on binary-size
  cost of `html5ever`.
- XML: **`roxmltree`** (read-only, very small) or **`quick-xml`**
  if we need streaming/emit.

Surface sketch:

```pluma
# Opaque — the actual DOM lives Rust-side.
alias html.document  = ...   # opaque
alias html.node      = ...   # opaque

# HTML parse is total — browser-style recovery.
def parse :: fun string -> html.document = built-in "html-parse"

# CSS-selector subset: tag, .class, #id, descendant, attribute.
def query     :: fun html.document string -> list html.node
def query-one :: fun html.document string -> option html.node

# Node inspection.
def tag        :: fun html.node -> string
def text       :: fun html.node -> string
def attr       :: fun html.node string -> option string
def children   :: fun html.node -> list html.node
def inner-html :: fun html.node -> string
def outer-html :: fun html.node -> string
```

XML mirrors this shape but `parse` returns `result xml.document
xml.error` because XML is strict.

Design notes:

- **DOM is opaque** (see Architectural pieces). Cheaper than
  translating an entire tree into Pluma values up front.
- **CSS selectors only, not XPath.** Selector subset matches what
  90% of scraping code needs.
- **No DOM mutation** in v1. Read-only. If users need to emit HTML,
  they build strings or use a separate templating module.

### Scripting (cluster of modules)

This is the biggest functional gap. Today `core.io` has flat file
I/O and argument access. To make Pluma a real shell-script
replacement we need:

#### `core.fs`

Path manipulation + structured filesystem ops. Sits on top of
`core.io`. No external crate needed — `std::path` + `std::fs` is
enough.

```pluma
# Path strings (no separate path type yet — keep it simple).
def join      :: fun string string -> string
def dirname   :: fun string -> string
def basename  :: fun string -> string
def extension :: fun string -> option string
def normalize :: fun string -> string
def absolute  :: fun string -> result string fs.error

# Directory ops.
def list-dir :: fun string -> result (list string) fs.error
def mkdir    :: fun string -> result nothing fs.error
def mkdir-p  :: fun string -> result nothing fs.error
def rmdir    :: fun string -> result nothing fs.error
def walk     :: fun string -> result (list string) fs.error

# Metadata.
def is-file :: fun string -> bool
def is-dir  :: fun string -> bool
def size    :: fun string -> result int fs.error
def mtime   :: fun string -> result time.instant fs.error

# Copy / move / temp.
def copy        :: fun string string -> result nothing fs.error
def move        :: fun string string -> result nothing fs.error
def temp-file   :: fun string -> result string fs.error    # arg = prefix
def temp-dir    :: fun string -> result string fs.error
```

#### `core.process`

Child-process spawn. Sync first; async wrapping arrives when ASYNC
lands.

Recommended crate: **`std::process`** is enough; **`subprocess`** or
**`duct`** for ergonomic stdin/stdout piping if we want it.

```pluma
alias process.output = {
    exit-code :: int
    stdout    :: string
    stderr    :: string
}

# Argv as a list — quoting/escaping is impossible to get wrong.
def run :: fun (list string) -> result process.output process.error =
    built-in "process-run"

# With explicit stdin input.
def run-with-input :: fun (list string) string -> result process.output process.error

# Spawn and return a handle for streaming.
def spawn :: fun (list string) -> result process.handle process.error
```

Important: `run` takes `list string`, not a single shell command
string. No shell-injection footguns by construction.

#### `core.env`

Promotes `args` and `env` out of `core.io`. Adds the rest.

```pluma
def args      :: fun nothing -> list string
def get       :: fun string -> option string             # env var
def set       :: fun string string -> nothing
def unset     :: fun string -> nothing
def cwd       :: fun nothing -> string
def chdir     :: fun string -> result nothing env.error
def home-dir  :: fun nothing -> option string
def hostname  :: fun nothing -> string
def pid       :: fun nothing -> int
```

#### `core.args`

Argument-parsing helpers — separate from `core.env.args` which is
just the raw list. Opt-parse style: flags, positional args,
subcommands, auto-generated help text.

Recommended crate: **`clap`** is overkill (huge binary); a custom
small parser in pure Pluma is more in keeping with the rest of the
library.

Design open: defer until we have records-with-defaults or a clear
pattern for "spec + parse → struct."

#### `core.glob`

Recommended crate: **`glob`**.

```pluma
def glob :: fun string -> result (list string) glob.error
```

`glob "src/**/*.pa"` returns matching paths.

#### `core.term`

Terminal helpers: ANSI colors, `is-tty`, terminal size.

Recommended crate: **`crossterm`** (or just hand-roll ANSI for the
basics).

```pluma
def is-tty       :: fun nothing -> bool
def size         :: fun nothing -> {rows :: int, cols :: int}
def red          :: fun string -> string     # wrap in ANSI red
def green        :: fun string -> string
def bold         :: fun string -> string
...
```

### `core.test`

Unit-testing framework. Bundled with a `pluma test` CLI subcommand.

Language addition required: a `test "<name>" { ... }` top-level
form. Three options:

- **(a) New top-level form** — `test "name" { body }`. Parser
  recognizes `test` keyword followed by a string literal and a
  block.
- **(b) Attribute on def** — `#[test] def my-test = fun { ... }`.
  Reuses existing def syntax; needs an attribute parser we don't
  have.
- **(c) Naming convention** — any `def` whose name starts with
  `test-` is a test. Simplest, but mixes test and non-test code in
  the namespace.

**Recommendation: (a).** Reads best, no attribute machinery needed,
the string is naturally the test name. Implementation: a new
`DefinitionKind::Test { name: String, body: ExprNode }`. The CLI's
`pluma test` discovers these across modules under the test root.

Surface:

```pluma
test "list.length on empty is zero" {
    assert.equals (list.length []) 0
}

test "json round-trip" {
    let v = json.object (map.from-entries [("x", json.int 1)])
    let s = json.stringify v
    assert.equals (json.parse s) (ok v)
}
```

`core.assert` provides:

```pluma
def equals         :: fun a a -> nothing where (ord a)   # via ord for diff output
def not-equals     :: fun a a -> nothing where (ord a)
def is-true        :: fun bool -> nothing
def is-false       :: fun bool -> nothing
def approx-equals  :: fun float float float -> nothing   # value, target, epsilon
def throws         :: fun (fun nothing -> a) -> nothing  # when we have throws
def snapshot       :: fun string a -> nothing            # snapshot test
```

Assertion failure throws (or, pre-throws, accumulates into the
test-runner's state). The CLI reports per-test pass/fail with diff
output for `equals` mismatches.

Snapshot tests write to `<test-file>.snap.yaml` next to the source,
with `--update` mode for accepting changes. Mirrors the pattern
already used by `insta` for Pluma's own tests.

Property-based testing (`core.gen` + `property "name" gen { ... }`)
is a follow-up, not v1.

## Other modules — domain sweep

Quick passes; each is one short section with recommended crate +
surface highlight. Detailed design happens when we get to each.

### Data formats

**`core.csv`** — recommended crate `csv` (BurntSushi). Read/write
rows, header-aware, configurable delimiter. Streaming variant later.

**`core.toml`** — recommended crate `toml`. Parse to a Pluma
`toml.value` ADT (similar shape to `json.value`).

**`core.url`** — recommended crate `url` (Servo). Parse, build,
percent-encode, query-string. Heavy edge cases; vendoring is
strongly preferred over hand-rolling.

**`core.base64`** — recommended crate `base64`. `encode`, `decode`,
standard + URL-safe variants.

**`core.hex`** — pure Pluma is fine; one-liner over `bytes`.

### Time

**`core.time`** — recommended crates `chrono` + `chrono-tz`.

```pluma
alias time.instant   = ...   # opaque
alias time.duration  = ...   # opaque or record { seconds :: int, nanos :: int }

def now           :: fun nothing -> time.instant
def monotonic     :: fun nothing -> time.instant
def parse-iso8601 :: fun string -> result time.instant time.error
def format-iso8601 :: fun time.instant -> string
def duration-ms   :: fun int -> time.duration
def add           :: fun time.instant time.duration -> time.instant
def diff          :: fun time.instant time.instant -> time.duration
def sleep         :: fun time.duration -> nothing       # sync; async variant via task
```

Question: keep `duration` opaque, or expose it as a record? Record
gives natural construction and is more in keeping with Pluma's
data-first feel.

### Networking — *depends on ASYNC.md*

**`core.tcp`**, **`core.udp`** — recommended `tokio::net` (post-
async).

**`core.http.client`** — recommended `reqwest` (async) or `ureq`
(sync). If we ship sync now, it gets replaced when ASYNC lands;
maybe worth waiting.

**`core.http.server`** — recommended `axum` or `hyper` directly.
Post-async only.

**`core.dns`** — `trust-dns-resolver` or `hickory-resolver`.

**`core.tls`** — `rustls` + `webpki-roots`. Bundled root certs; no
OpenSSL.

**`core.ws`** — `tungstenite` (sync) / `tokio-tungstenite` (async).

### Concurrency — *depends on ASYNC.md*

**`core.channel`** — bounded/unbounded MPSC. Built on top of
`task`'s scheduler — same crate (`tokio::sync::mpsc` if we go that
route).

**`core.semaphore`**, **`core.mutex`** — single-threaded but still
useful for resource gating (limit DB connections, throttle outgoing
HTTP, etc.).

**`core.timer`** — `after`, `every`, deadlines. Wraps `tokio::time`.

### Cryptography

**`core.hash`** — recommended crates `sha2`, `blake3`. SHA-256/512,
BLAKE3, plus MD5/SHA-1 for legacy compat (clearly labeled).

**`core.hmac`** — `hmac`.

**`core.random`** — `rand` + `rand_chacha`. Secure and seedable
variants both exposed. Seedable PRNG state is an opaque handle.

**`core.uuid`** — `uuid` crate. v4 (random) + v7 (timestamp-ordered).

**`core.crypto`** — symmetric/asymmetric encryption. `ring` or
`age` (high-level format). Defer to v2; not v1.

### Collections beyond `list`/`map`

**`core.set`** — hash set on the same machinery as `map`.

**`core.tree-map`**, **`core.tree-set`** — sorted-by-key. BTreeMap
under the hood. Useful when iteration order matters.

**`core.deque`** — `VecDeque`-backed.

**`core.heap`** — binary heap / priority queue.

### Text

**`core.regex`** (extend current) — add capture groups, replace,
named groups. Recommended: keep using `regex` crate; expand the
builtin surface.

**`core.unicode`** — recommended crates `unicode-segmentation`,
`unicode-normalization`. Grapheme iteration, NFC/NFD normalization,
char categories.

**`core.fmt`** — printf-style formatting OR builder pattern. Likely
interacts with language-level string interpolation if/when that
lands. Defer until interpolation is decided.

### Logging

**`core.log`** — levels (debug/info/warn/error), structured fields,
configurable sinks (stderr, file, JSON output to stdout for
log-aggregator pipelines). Recommended crate `tracing` (more
structured) or just `log` (simpler). Lean toward `tracing` because
it composes with the async story.

### Numerics

**`core.bigint`** — `num-bigint`. Arbitrary-precision integers. The
existing `numeric` trait should be implementable on `bigint`.

**`core.stats`** — pure Pluma. Mean, median, variance, stdev,
percentile.

**`core.rational`** — `num-rational`. v2 — niche.

**`core.complex`** — `num-complex`. v2 — niche.

### Compression / archives

**`core.gzip`** — `flate2`. `compress`, `decompress` over `bytes`.

**`core.zip`** — `zip`. Archive read + write.

**`core.tar`** — `tar`. Archive read + write.

### Database / storage

**`core.sqlite`** — `rusqlite` with `bundled` feature. The single
biggest "battery" for scripting use cases.

```pluma
alias sqlite.connection = ...   # opaque

def open    :: fun string -> result sqlite.connection sqlite.error
def close   :: fun sqlite.connection -> nothing
def execute :: fun sqlite.connection string (list sqlite.value) -> result int sqlite.error
def query   :: fun sqlite.connection string (list sqlite.value) -> result (list sqlite.row) sqlite.error

# Transactions as a `with` block (once we have it from ASYNC).
def transaction :: fun sqlite.connection (fun sqlite.connection -> result a sqlite.error) -> result a sqlite.error
```

Prepared statements are an internal optimization — exposed via a
builder API later if/when needed.

**`core.kv`** — small file-backed key-value store. Probably `sled`
or `redb`. Defer — SQLite covers this use case for now.

### Developer ergonomics

**`core.debug`** — `pretty-print : fun a -> string` for any value.
`dump : fun a -> a` for inline tracing (prints + returns).
Implementation reuses the `Debug` impls that already exist in the
VM in debug builds.

**`core.assert`** — runtime assertions in normal code (distinct
from `core.test.assert` even if surface looks similar). `assert.is-
true`, `assert.equals`, with a clear panic-on-failure mode.

**`core.panic`** — `panic`, `unreachable`, `todo`. Each takes a
message; each terminates with a stack trace if we have one.

## Sequencing

What depends on what. Build in this order to maximize unlock-per-
work.

### Tier 0 — prerequisites

1. ~~**Stdlib `.pa` infrastructure** (type annotations, `built-in`
   expr, stdlib `.pa` loader).~~ **Shipped.** Modules are now authored
   as `.pa` files under `compiler/src/stdlib/`.
2. **Opaque-handle design note.** Short follow-up. Unblocks any
   module that holds non-decomposable state (sqlite connections, DOM
   trees, etc.). Still outstanding.

### Tier 1 — no further blockers (build any time)

These are pure transforms over data already in Pluma. No async, no
opaque state beyond simple wrappers.

- `core.json` — single biggest unblock for real apps.
- `core.csv`, `core.toml`, `core.base64`, `core.url`, `core.hex`
- `core.fs`, `core.env`, `core.process` (sync), `core.glob`,
  `core.term`
- `core.hash`, `core.hmac`, `core.uuid`
- `core.random` (sync API)
- `core.regex` extensions
- `core.set`, `core.tree-map`, `core.deque`, `core.heap`
- `core.unicode`
- `core.bigint`
- `core.gzip`, `core.zip`, `core.tar`
- `core.sqlite`
- `core.debug`, `core.assert`, `core.panic`
- `core.log` (sync, stderr-only sink)
- `core.html`, `core.xml`

### Tier 2 — language feature unlock

These need a language addition first.

- **`core.test`** — needs the `test "name" { ... }` top-level form.
- **`core.fmt`** — wants string interpolation, or at least a
  syntactic story for format specs.

### Tier 3 — wait for ASYNC.md

These shape badly without `task`. Don't ship sync stubs that get
thrown away.

- `core.http.client`, `core.http.server`
- `core.ws`, `core.tcp`, `core.udp`, `core.dns`
- `core.tls` (technically standalone, but mostly used with HTTP)
- `core.channel`, `core.semaphore`, `core.mutex`, `core.timer`
- `core.time.sleep`'s async variant (sync `sleep` ships Tier 1)

### Suggested first push

If we're prioritizing demo-ability (JSON, unit testing, and `assert`
have already shipped):

1. `fs`, `process`, `env`, `glob`, `term` — together this makes Pluma
   a credible scripting language. The biggest remaining gap.
2. SQLite — turns Pluma into a real "small app" language.
3. HTML — unlocks scraping demos.
4. Logging, debug, panic — quality-of-life.

Everything else comes after.

## Open questions

- **JSON number representation.** Split `int`/`float` (proposed
  above) vs single `number float`. Recommendation is split; not
  locked in.
- **Opaque-handle representation.** Single `Value::Opaque(Rc<dyn
  Any>)` vs per-domain variants. Decide in the follow-up design
  note before the first opaque-holding module lands.
- **`core.test` form.** New top-level keyword (`test "name" {
  ... }`) vs attribute vs naming convention. Recommendation: new
  keyword. Confirm before implementing.
- **Async-stub policy.** For HTTP and friends, do we ship sync now
  and replace, or wait for ASYNC? Recommendation: wait. The sync
  surface and the async surface diverge too much; better to
  introduce them right than refactor users.
- **`core.fmt` vs string interpolation.** If language-level
  interpolation lands, `fmt` shrinks dramatically. Sequence these
  together.
- **Crate-vetting / license policy.** Are we strict MIT/Apache?
  How do we audit transitive deps? Worth a one-page policy when
  the first heavy crate (probably `rusqlite` or `html5ever`) lands.
- **Namespace tier.** Stay flat under `core.*`, or split
  `core.*` (essentials) vs `std.*` (rest)? Defer until surface
  area starts hurting.
- **Doc-comment plumbing.** Whether doc comments above defs are
  attached to their defs in the typed AST and surfaced as LSP hovers —
  wire it up if it's not already done.
