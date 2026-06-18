# Numbers and math

Pluma has two number types: `int` for whole numbers and `float` for decimals. The
[operators](/docs/reference/operators) page covers the arithmetic that works on
both: `+`, `-`, `*`, `/`, `%`. This page is about `std/math`: rounding, roots,
logarithms, trigonometry, and the conversions for crossing between the two number
types.

## int and float stay separate

Pluma keeps whole numbers and decimals apart and won't quietly mix them, so
`2 + 3.5` is a type error, not a silent conversion. When you do need to cross
between them, you say so:

```pluma
use std/math

math.to-float 3     # => 3.0    (int → float)
math.to-int 3.9     # => 3      (float → int, toward zero)
math.to-int -3.9    # => -3
```

`to-int` drops the fractional part toward zero; it doesn't round. (For rounding,
see below.) This conversion matters more than it looks: division with `/` on two
ints is *integer* division, so `7 / 2` is `3`. When you want a real quotient,
convert first: `(math.to-float 7) / 2.0` is `3.5`.

## Rounding

Three functions turn a `float` into the nearest `int`, differing only in which
direction they go. All three return an `int`:

```pluma
use std/math

math.floor 3.7    # => 3     (down toward negative infinity)
math.ceil 3.2     # => 4     (up toward positive infinity)
math.round 2.5    # => 3     (to nearest; ties go up)
```

Watch the difference on negatives: `math.floor -3.2` is `-4` (further down), while
`math.ceil -3.7` is `-3` (toward zero). `floor` and `ceil` always move the same
direction regardless of sign.

## Roots, powers, and logarithms

```pluma
use std/math

math.sqrt 9.0       # => 3.0
math.sqrt 2.0       # => 1.4142135623730951
math.exp 0.0        # => 1.0     (e raised to the power)
math.log math.e     # => 1.0     (natural log)
math.log10 1000.0   # => 3.0
math.log2 8.0       # => 3.0
```

`exp` and the `log` family are inverses: `exp` raises `e` to a power, and `log` is
the natural logarithm (base `e`), with `log10` and `log2` for the two other bases
you reach for most. These all take and return a `float`.

## Trigonometry

`sin`, `cos`, and `tan` work on `float`s and measure angles in radians:

```pluma
use std/math

math.sin 0.0    # => 0.0
math.cos 0.0    # => 1.0
```

The constant `math.pi` is on hand for converting degrees to radians, and
`math.e` is Euler's number:

```pluma
math.pi   # => 3.141592653589793
math.e    # => 2.718281828459045
```

## Absolute value

`math.abs` gives the magnitude of a whole number, dropping its sign:

```pluma
math.abs -5   # => 5
math.abs 5    # => 5
```

## See also

- **[Operators](/docs/reference/operators)**: the arithmetic and comparison
  operators, their precedence, and the no-implicit-promotion rule.
- **[Working with strings](/docs/stdlib/strings)**: `string.to-int` and
  `string.to-float` for turning text into numbers.
