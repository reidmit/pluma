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
