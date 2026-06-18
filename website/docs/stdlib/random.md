# Randomness and IDs

Two small modules cover the times you need an unpredictable value: `std/random`
for random numbers, bytes, and choices, and `std/uuid` for unique identifiers.

## Random numbers

`std/random` draws from the operating system's *secure* source of randomness, so
the results are safe even where unpredictability really matters — passwords,
tokens, session ids. Every call returns a fresh, independent value, so these
functions don't have a fixed result to show:

```pluma
use std/random

random.int ()     # a different large whole number every call
random.float ()   # a different decimal in [0, 1) every call
random.bool ()    # true about half the time — a coin flip
```

(A *seedable* generator — the kind that replays the same sequence every run, handy
for tests and reproducible simulations — isn't available yet, but is planned.)

### Ranges, choices, and bytes

Three more functions cover the common needs, and each guards its edge case with a
[`result`](/docs/reference/errors) or [`option`](/docs/reference/errors) rather
than misbehaving. `int-range` picks a whole number in a half-open range — `low`
included, `high` excluded:

```pluma
use std/random

random.int-range 1 7   # ok n, where n is 1 through 6 (a die roll)
random.int-range 5 5   # => err "random.int-range: low (5) >= high (5)"
```

`choice` picks a random element of a list, and returns `none` when there's nothing
to pick from:

```pluma
random.choice ["heads", "tails"]   # some "heads" or some "tails"
random.choice []                   # => none
```

And `random.bytes n` hands back `n` cryptographically-random bytes (as a `result`,
since a negative length makes no sense) — the raw material for a token or a salt.

## UUIDs

A UUID is a 128-bit identifier that looks like
`550e8400-e29b-41d4-a716-446655440000`. Pluma represents one as a plain lowercase,
hyphenated **string** — there's no special UUID type to learn, so every
[`std/string`](/docs/stdlib/strings) operation works on it directly.

Two versions, for two jobs:

```pluma
use std/uuid

uuid.v4 ()   # e.g. "f47ac10b-58cc-4372-a567-0e02b2c3d479"  — fully random
uuid.v7 ()   # e.g. "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b"  — timestamp-ordered
```

`v4` is fully random, which makes it a good fit for a token or session id that
shouldn't be guessable. `v7` is still unguessable but newer ids sort *after*
older ones, which makes it well-behaved as a database key — ordered inserts are
faster.

### Checking one

When a UUID arrives from outside — a request, a URL — you'll want to confirm it's
real. `is-valid` answers yes or no, and `parse` checks it and returns it in
canonical lowercase form, failing with a `result` if it isn't a UUID at all:

```pluma
use std/uuid

uuid.is-valid "550e8400-e29b-41d4-a716-446655440000"   # => true
uuid.is-valid "nope"                                   # => false
uuid.parse "not-a-uuid"                                # => err "failed to parse a UUID"
```

## See also

- **[Working with strings](/docs/stdlib/strings)** — a UUID is just a string, so
  the string toolkit applies.
- **[Bytes](/docs/reference/bytes)** — the raw bytes from `random.bytes`.
