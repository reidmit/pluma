# calc — an expression calculator

A small interactive calculator, written entirely in Pluma. It reads one
expression per line, evaluates it, and prints the result — keeping
user-defined variables around between lines until you quit.

```
pluma run examples/calc
```

It exists to exercise corners of the language the other examples don't:
an **interactive stdin loop** (read until EOF), **mutable state** (the
variable environment lives in a `ref`), a **numeric tower** that mixes
`int` and `float`, and the prelude **`numeric` trait** (`+ - * /` are
overloaded over both types).

## A session

```
> x = 3 * (4 + 1)
= 15
> sqrt(x) + 1
= 4.872983346207417
> 2 + 3.5
= 5.5
> area = pi * r ^ 2
error: undefined name 'r'
> r = 2
= 2
> area = pi * r ^ 2
= 12.566370614359172
> :vars
area = 12.566370614359172
e = 2.718281828459045
pi = 3.141592653589793
r = 2
x = 15
```

Each line is prompted with `> `; an evaluation result is printed with a
`= ` prefix. A parse or evaluation error reports and the loop continues
— one bad line never ends the session. Press Ctrl-D (EOF) or type `:q`
to quit.

## What it understands

**Operators** (lowest to highest precedence): `+ -`, then `* / %`, then
`^` (exponent, right-associative), then unary `-`. Parentheses group.

**Numbers** are integers or floats. Arithmetic stays an integer until a
float or a division forces promotion — so `2 + 3` is `5`, `2 + 3.5` is
`5.5`, and `7 / 2` is `3.5`. `^` keeps the base's type for integer
exponents (`2 ^ 10` is `1024`), but a negative or fractional exponent
yields a float.

**Functions** are called with parentheses:

| Form | Functions |
| - | - |
| `f(x)` | `sqrt sin cos tan ln log log2 exp abs floor ceil round int float` |
| `f(x, y)` | `pow(b, e)` · `max(a, b)` · `min(a, b)` · `log(x, base)` |

`log` is overloaded by arity: one argument is base-10, two is log-base-y.

**Variables** persist across lines (`name = expr`); `pi` and `e` are
predefined. **Commands:** `:vars` lists bindings (sorted by name), `:help`
prints a cheat-sheet, `:q` quits.

## Layout

| File | Role |
| - | - |
| `value.pa` | The `num` tower: the `int`/`float` type plus the promotion rules the `numeric` trait won't do (`2 + 3.5`), and `^`, which Pluma has no operator for. |
| `lexer.pa` | `string` → `list token`: a byte-at-a-time scanner for numbers, names, and single-character operators. |
| `parser.pa` | `list token` → `stmt`: a precedence-climbing parser. Each step consumes a prefix of the token list and returns the remainder. |
| `eval.pa` | `expr` → `result num`: walks the tree against the environment, dispatching operators to the tower and resolving function calls. |
| `main.pa` | The REPL: seed `pi`/`e`, read-eval-print until EOF, route `:`-commands, and persist assignments into the `ref` environment. |

## How it works

Each line runs through three stages, mirroring the file layout:

1. **Tokenize** (`lexer.pa`) — every marker is ASCII, so the scanner
   walks the line one character at a time, emitting `t-num`, `t-name`,
   and `t-op` tokens.
2. **Parse** (`parser.pa`) — a classic precedence ladder
   (`expr → term → power → unary → atom`) folds the tokens into an
   `expr` tree, threading the unconsumed tokens through each step.
3. **Evaluate** (`eval.pa`) — a recursive walk returning
   `result num string`, so the first undefined name, divide-by-zero, or
   bad function call short-circuits with a message the REPL prints.

The variable environment is a `ref (map string num)` threaded through
evaluation. Only the REPL writes to it (on assignment); evaluation just
reads — so the same cell carries state from one line to the next.
