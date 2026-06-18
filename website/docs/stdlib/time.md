# Time and durations

`std/time` handles two related but distinct ideas, and keeping them straight is
the key to the whole module:

- An **instant** is a single point on the wall clock: "this moment", the time a
  log line was written. It's what you compare and format.
- A **duration** is a *length* of time ("three days", "500 milliseconds") with
  no particular start. It's what you add to an instant, or get back from
  subtracting two.

Both are opaque types: you build and read them through the functions here rather
than poking at a raw number, so there's never any doubt about units. Everything
works in UTC; named time zones aren't available yet.

## Durations

A duration has two ways in. The quickest is a **duration literal**: a number
glued to a unit, which is ordinary language syntax and needs no import:

```pluma
5ms       # five milliseconds
2m20s     # two minutes and twenty seconds
3h        # three hours
```

These are the same durations you pass to [`task.sleep`](/docs/reference/concurrency)
and `task.with-timeout`. When the amount is computed rather than written out, the
unit builders in `std/time` do the same job from an `int`:

```pluma
use std/time

time.seconds 2     # same kind of value as the literal 2s
time.minutes 90
time.days 3
```

Read a duration back in whole units with the `as-*` family, and combine durations
with `duration-add`, `scale`, and friends:

```pluma
use std/time

time.as-seconds (time.minutes 2)   # => 120
time.as-millis 500ms               # => 500
```

## Reading the clock

`time.now ()` reads the current instant; `time.sleep` pauses the program for a
duration. Because the clock advances on its own, a call to `now` doesn't have a
fixed result:

```pluma
use std/time

time.now ()                    # the current instant, different every call
time.sleep (time.seconds 2)    # wait two seconds
```

For *measuring* how long something took, reach for `time.monotonic ()` instead. It
reads a steady, always-forward clock and returns a duration, so subtracting two
readings gives a reliable elapsed time even if the wall clock is adjusted
underneath you.

## Building a specific instant

To name a particular date, use `time.date` (just the day) or `time.date-time`
(down to the second). They *validate* the components, so they return a
[`result`](/docs/reference/errors): an impossible date is an `err`, not a
silently mangled instant:

```pluma
use std/time

time.date 2026 5 25          # => ok (an instant for 2026-05-25)
time.make 2026 2 30 0 0 0 0  # => err (time-error.field-out-of-range "day" ...)
```

There are also `from-unix`, `from-unix-millis`, and `from-unix-nanos` for when
you already have an epoch timestamp.

## Calendar fields

An instant is a point in time, not a set of calendar numbers. To read its year,
month, day, and so on, explode it into `parts` with `time.to-parts`:

```pluma
use std/time

let p = time.to-parts t   # p.year, p.month, p.day, p.hour, ... ; p.weekday is 1–7
```

`parts` is a plain record, so you read its fields with a dot. Hand one back to
`time.from-parts` to go the other way.

## Formatting and parsing

`time.format` renders an instant with a `strftime`-style pattern, and `to-iso`
gives the standard ISO-8601 string you'd put in a log or send over the wire:

```pluma
use std/time

time.format t "%Y-%m-%d"   # => "2026-05-25"
time.format t "%A, %B %d"  # => "Monday, May 25"
time.to-iso t              # => "2026-05-25T14:30:00Z"
```

Parsing runs the other way and can fail, so it returns a `result`:

```pluma
time.parse-iso "2026-05-25T14:30:00Z"   # => ok (an instant)
time.parse-iso "nope"                    # => err (time-error.unparseable "nope")
```

## Arithmetic on instants

Adding a duration to an instant gives another instant; subtracting two instants
gives the duration between them:

```pluma
use std/time

time.add t (time.days 7)               # one week later
time.as-hours (time.diff (time.add-days t 1) t)   # => 24
```

The `add-days`, `add-hours`, `add-minutes`, and `add-seconds` helpers skip the
duration step for the common cases. `add-months` and `add-years` are special:
shifting a calendar month can land on a day that doesn't exist (January 31 plus a
month), so they return a `result`. To compare instants, use `before`, `after`, or
`compare`, and `time.min`/`time.max` to pick the earlier or later of two.

## See also

- **[Concurrency](/docs/reference/concurrency)**: `task.sleep` and timeouts take
  the same durations.
- **[Errors and missing values](/docs/reference/errors)**: the `result`s from
  date construction and parsing, and how `time-error` erases into `std/error`.
