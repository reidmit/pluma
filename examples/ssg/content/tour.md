---
title: A short tour of Pluma
---

# A short tour

A whirlwind through the features that make Pluma *Pluma*.

## Enums and pattern matching

Data is modeled with `enum`s, and `when` matches over them:

```
enum shape {
	circle float
	rect float float
}

def area = fun s {
	when s is circle r { 3.14159 *. r *. r }
	is rect w h { w *. h }
}
```

There is no `null`. Absence is `option`, and fallible work returns
`result` — both are just ordinary enums.

## Traits

Traits give you ad-hoc polymorphism with dictionary passing:

```
trait showable a {
	show :: a -> string
}
```

Implement it once per type and `show` works everywhere a `showable`
constraint is in scope.

## Lists

Lists support literals, spreads, and rest-patterns:

- build with `[1, ...xs, 9]`
- destructure with `[head, ...tail]`
- transform with `map`, `filter`, and `fold`

## A worked example

Summing a list, the functional way:

```
def sum = fun xs {
	when xs is [] { 0 }
	is [n, ...rest] { n + sum rest }
}
```

That's the whole language in miniature: **small pieces**, composed.

---

Back to the [home page](index.html).
