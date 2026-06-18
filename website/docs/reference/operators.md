# Operators

Arithmetic and comparison operators are overloaded over `int` and `float`
through traits — the same `+` works on both, and a generic `fun x { x + x }`
works for either. There is no dotted float operator set, and no implicit
promotion.

## Arithmetic

| Operator | Signature | Meaning |
| --- | --- | --- |
| `+` | `numeric a => a a -> a` | Addition |
| `-` | `numeric a => a a -> a` | Subtraction, and unary negation |
| `*` | `numeric a => a a -> a` | Multiplication |
| `/` | `numeric a => a a -> a` | Division — integer division on int, true division on float |
| `%` | `int int -> int / float float -> float` | Remainder |

The two operands must have the same type, so `2 + 3.5` is a type error rather
than a silent promotion. `%` is not a trait method; it resolves to int or float
by its operands.

## Comparison

The ordering operators are shorthand for `compare` (the `ord` trait) plus a check
on the result, so they work on any ordered type. `==` and `!=` compare any two
values of the same type structurally.

| Operator | Signature | Meaning |
| --- | --- | --- |
| `<` | `ord a => a a -> bool` | Less than |
| `>` | `ord a => a a -> bool` | Greater than |
| `<=` | `ord a => a a -> bool` | Less than or equal |
| `>=` | `ord a => a a -> bool` | Greater than or equal |
| `==` | `a a -> bool` | Structural equality |
| `!=` | `a a -> bool` | Structural inequality |

## Logical, string, and coalesce

| Operator | Signature | Meaning |
| --- | --- | --- |
| `and` | `bool bool -> bool` | Logical and (short-circuiting) |
| `or` | `bool bool -> bool` | Logical or (short-circuiting) |
| `++` | `string string -> string` | String concatenation |
| `??` | `(option a) a -> a` | The contained value if some/ok, else the right-hand default |

The coalesce operator `??` also works on `result`. It is lazy in its right
operand and right-associative, so defaults chain: `a ?? b ?? c`. It is the
recovering dual of `try`.

## Bitwise

These treat an `int` as 64 two's-complement bits. They are int-only, and each is
also a function in `std/bit` for use in pipelines.

| Operator | Signature | Meaning |
| --- | --- | --- |
| `&` | `int int -> int` | Bitwise and |
| `\|` | `int int -> int` | Bitwise or |
| `^` | `int int -> int` | Bitwise xor |
| `<<` | `int int -> int` | Shift left (zero-fill) |
| `>>` | `int int -> int` | Shift right, arithmetic (sign-preserving) |
| `>>>` | `int int -> int` | Shift right, logical (zero-fill) |
| `~` | `int -> int` | Bitwise not (prefix) |

Unlike C, the bitwise operators bind *tighter* than comparison, so
`x & mask == 0` parses as `(x & mask) == 0`. Among themselves: shifts tightest,
then `&`, then `^`, then `|`.

## Pipe

The pipe `|>` threads its left operand in as the *first* argument of the call on
its right, so `x |> f a b` is `f x a b`. It binds looser than everything else and
is left-associative, so a chain reads top to bottom:

```pluma
[1, 2, 3, 4, 5]
	|> list.filter fun n { n > 2 }
	|> list.map fun n { n * 10 }
	|> print
```
