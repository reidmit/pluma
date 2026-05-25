+++
title = "Bytes"
description = "An immutable sequence of 8-bit values — no UTF-8 invariant."
weight = 2
+++

Use `bytes` for binary data, wire formats, hashes, and anything else that isn't necessarily text. Literal syntax uses single quotes:

```pluma
let greeting    = 'hello'
let png-header  = '\x89PNG\r\n\x1a\n'
let empty-bytes = ''
```

## Distinct from `string`

`bytes` and `string` are distinct types with no implicit conversion. The bridge is explicit:

- `string.to-bytes :: string -> bytes` — UTF-8 encode (infallible).
- `bytes.to-string :: bytes -> result string string` — UTF-8 decode (fallible).

## Literal syntax

Bytes literals support `\\`, `\'`, `\0`, `\t`, `\r`, `\n`, and `\xNN` (two hex digits). They do **not** support `$(…)` interpolation — interpolation lives on `string`. Non-ASCII characters that appear inside a bytes literal are encoded as their UTF-8 bytes.

## In patterns

`'…'` patterns work in `when` / `if` / `while`:

```pluma
when method is 'GET' { ... }
is 'POST' { ... }
```

## The `core.bytes` module

A parallel surface to `core.string`:

```pluma
use core.bytes

bytes.length b              # int — byte count
bytes.is-empty b            # bool
bytes.at b i                # option int — none if out of bounds
bytes.concat a b
bytes.slice b start end     # clamp-to-bounds; end < start gives ''
bytes.contains haystack needle
bytes.starts-with b prefix
bytes.ends-with b suffix
bytes.repeat b n
bytes.reverse b
bytes.to-list b             # list int — one entry per byte
bytes.from-list xs          # result bytes string — errs if any int is < 0 or > 255
bytes.join parts sep        # parts :: list bytes
bytes.split b sep           # empty sep splits into single-byte chunks
```

## Hashing and ordering

`bytes` has prelude `hash` and `ord` instances, so `compare 'abc' 'abd'` and using bytes as map keys both work without any setup. Ordering is byte-lexicographic.

## Byte-aware I/O

Byte-side I/O lives in `core.io`:

```pluma
io.read-file-bytes path        # result bytes string — survives non-UTF-8 contents
io.write-file-bytes path bytes # result nothing string
io.append-file-bytes path bytes
io.read-all-bytes ()           # drain stdin as bytes
io.write-bytes b               # raw write to stdout, no newline, no Display formatting
io.write-err-bytes b           # same, to stderr
```

The text-side equivalents (`io.read-file`, `io.write-file`, `io.read-all`) still exist and still require UTF-8; reach for the byte-side versions when you're dealing with binary data or when source encoding is uncertain.
