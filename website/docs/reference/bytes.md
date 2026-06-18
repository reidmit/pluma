# Bytes

Use `bytes` for binary data — wire formats, hashes, anything that isn't
necessarily text. Literals use single quotes:

```pluma
let greeting   = 'hello'
let png-header = '\x89PNG\r\n\x1a\n'
let empty      = ''
```

Literals support `\\`, `\'`, `\0`, `\t`, `\r`, `\n`, and `\xNN`. They do not
interpolate — `$(...)` lives on `string`.

## Distinct from string

Bytes and strings are different types with no implicit conversion. The bridge is
explicit — encoding is infallible, decoding can fail:

```pluma
string.to-bytes :: fun string -> bytes
bytes.to-string :: fun bytes -> result string string
```

## The std/bytes module

A parallel surface to std/string, plus list conversions:

```pluma
use std/bytes

bytes.length b           # int — byte count
bytes.at b i             # option int — none if out of bounds
bytes.slice b start end  # clamp-to-bounds
bytes.concat a b
bytes.to-list b          # list int
bytes.from-list xs       # result bytes string
bytes.split b sep
bytes.join parts sep
```

Bytes have prelude `hash` and `ord` instances (ordering is byte-lexicographic),
so they work as map keys with no setup. Byte-aware file I/O lives in `std/sys/fs`
(`fs.read-file-bytes`, `fs.write-file-bytes`), and survives non-UTF-8 contents.
