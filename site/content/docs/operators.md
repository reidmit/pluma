+++
title = "Operators"
description = "Binary operators are not overloadable. Integer and float operators are distinct."
weight = 4
+++

## Logical

| Operator | Signature | Meaning |
| - | - | - |
| `&&` | `bool bool -> bool` | Logical and |
| `\|\|` | `bool bool -> bool` | Logical or |

## Integer arithmetic

| Operator | Signature | Meaning |
| - | - | - |
| `+` | `int int -> int` | Addition |
| `-` | `int int -> int` | Subtraction |
| `*` | `int int -> int` | Multiplication |
| `/` | `int int -> int` | Division |
| `%` | `int int -> int` | Remainder |

## Float arithmetic

Float operators carry a trailing dot to keep them disjoint from the integer set.

| Operator | Signature | Meaning |
| - | - | - |
| `+.` | `float float -> float` | Addition |
| `-.` | `float float -> float` | Subtraction |
| `*.` | `float float -> float` | Multiplication |
| `/.` | `float float -> float` | Division |
| `%.` | `float float -> float` | Remainder |

## String

| Operator | Signature | Meaning |
| - | - | - |
| `++` | `string string -> string` | Concatenation |

## Option

| Operator | Signature | Meaning |
| - | - | - |
| `??` | `(option a) a -> a` | Coalesce — yield the left value if `some`, else the right. |

{% note() %}
Operators are not overloadable. For overloaded numeric behavior (e.g., `add` on either `int` or `float`), see the `numeric` trait on the [Traits](@/docs/traits.md) page.
{% end %}
