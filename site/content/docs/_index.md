+++
title = "Language reference"
description = "A small, statically-typed functional language."
sort_by = "weight"
template = "docs/section.html"
page_template = "docs/page.html"
+++

Pluma is an immutable-by-default functional language with Hindley–Milner type inference, nominal enums, row-polymorphic records, and traits for ad-hoc polymorphism. This reference covers the language as it currently exists; the standard library lives separately.

## At a glance

```
def main = fun {
    let nums = [1, 2, 3, 4]
    let doubled = map nums fun n { n * 2 }

    when (sum doubled) is total {
        print "total: $(to-string total)"
    }
}
```

Key characteristics:

- **Pure by default.** The only mutation primitive is `ref`; everything else is immutable.
- **Uncurried.** `add 5` is an arity error, not partial application. Wrap in a `fun` to partially apply.
- **Pattern-driven control flow.** `if`, `when`, and `while` all destructure with patterns; `when` is exhaustiveness-checked.
- **Nominal enums, structural records.** Records are row-polymorphic; enums have module-qualified identity.
- **Traits for overloading.** Trait methods dispatch on argument type and can carry `where` constraints.
