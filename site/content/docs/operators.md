+++
title = "Operators"
description = "Arithmetic and comparison operators are overloaded over int and float through the numeric and ord traits."
weight = 4
+++

## Logical

| Operator | Signature | Meaning |
| - | - | - |
| `&&` | `bool bool -> bool` | Logical and |
| `\|\|` | `bool bool -> bool` | Logical or |

## Arithmetic

`+`, `-`, `*`, and `/` are overloaded over `int` and `float` through the
[`numeric`](@/docs/traits.md) trait — the same operator works on both types,
and a generic `def double = fun x { x + x }` stays polymorphic (`numeric a => a -> a`).
There is no separate dotted float operator set.

| Operator | Signature | Meaning |
| - | - | - |
| `+` | `numeric a => a a -> a` | Addition |
| `-` | `numeric a => a a -> a` | Subtraction (and unary negation) |
| `*` | `numeric a => a a -> a` | Multiplication |
| `/` | `numeric a => a a -> a` | Division (integer division on `int`, true division on `float`) |
| `%` | `int int -> int` / `float float -> float` | Remainder |

The two operands must have the same type — there is no implicit int/float
promotion, so `2 + 3.5` is a type error. `%` is not a `numeric` method; it
resolves to int or float by the operand types.

## Comparison

`<`, `>`, `<=`, and `>=` desugar to [`ord`](@/docs/traits.md) `compare` plus a
check against the resulting `ordering` variant, so they work on any type with
an `ord` instance (`int`, `float`, `string`, and parametrically `option`/`result`).
`==` and `!=` compare any two values of the same type structurally.

| Operator | Signature | Meaning |
| - | - | - |
| `<` | `ord a => a a -> bool` | Less than |
| `>` | `ord a => a a -> bool` | Greater than |
| `<=` | `ord a => a a -> bool` | Less than or equal |
| `>=` | `ord a => a a -> bool` | Greater than or equal |
| `==` | `a a -> bool` | Structural equality |
| `!=` | `a a -> bool` | Structural inequality |

## String

| Operator | Signature | Meaning |
| - | - | - |
| `++` | `string string -> string` | Concatenation |

## Bitwise

These treat an `int` as a flat row of 64 bits (two's complement). They are
`int`-only — there is no `float` overload and no trait dispatch — and each is
also available as a function in `std.bit` for use in `|>` chains or as a
first-class value.

| Operator | Signature | Meaning |
| - | - | - |
| `&` | `int int -> int` | Bitwise and |
| `\|` | `int int -> int` | Bitwise or |
| `^` | `int int -> int` | Bitwise xor |
| `<<` | `int int -> int` | Shift left (zero-fill) |
| `>>` | `int int -> int` | Shift right, arithmetic (sign-preserving) |
| `>>>` | `int int -> int` | Shift right, logical (zero-fill) |
| `~` | `int -> int` (prefix) | Bitwise not (flip every bit) |

Unlike C, the bitwise operators all bind **tighter than comparison**, so the
common idiom `x & mask == 0` parses as `(x & mask) == 0` rather than the
surprising `x & (mask == 0)`. Among themselves they follow the familiar order:
shifts tightest, then `&`, then `^`, then `|` — and all of them looser than
`+`/`-`.

## Pipe

`|>` threads its left operand in as the **first** argument of the call on its
right, so `x |> f` is `f x` and `x |> f a b` is `f x a b`. It binds looser than
every other operator and is left-associative, so a chain reads top-to-bottom:

```pluma
[1, 2, 3, 4, 5]
	|> list.filter fun n { n > 2 }
	|> list.map fun n { n * 10 }
	|> print
```

is `print (list.map (list.filter [1, 2, 3, 4, 5] (fun n { n > 2 })) (fun n { n * 10 }))`.

## Coalesce

| Operator | Signature | Meaning |
| - | - | - |
| `??` | `(option a) a -> a` / `(result a e) a -> a` | Yield the contained value if `some`/`ok`, else the right-hand default. |

`??` is lazy in its right operand and right-associative, so defaults chain:
`a ?? b ?? c`. It is the recovering dual of `try`.

{% note() %}
Arithmetic and comparison operators dispatch through traits; see the
[Traits](@/docs/traits.md) page for the `numeric`, `ord`, and `hash`
declarations and how to add instances for your own types.
{% end %}
