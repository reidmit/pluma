# Styling with CSS

`std/css` lets you write styles as typed Pluma values instead of strings. You
build colors, lengths, and declarations from functions, so a unit you can't typo
and a property name the compiler knows replace a hand-written `"color: red;
padding: 1rem"`. It pairs with [`std/view`](/docs/stdlib/view): the same values
serialize to identical CSS on the server and in the browser.

There are two ways to apply styles — an inline `style` string, or a reusable
scoped class — and both are built from the same typed values.

## Typed values and declarations

A **declaration** is one property set to one value, like `padding: 1rem`. You
build it by handing a typed value to a property function:

```pluma
use std/css

css.color (css.hex "#e11")        # color: #e11
css.padding (css.rem 1.0)         # padding: 1rem
css.background (css.rgb 37 99 235)# background: rgb(37, 99, 235)
css.width (css.pct 50.0)          # width: 50%
```

Colors come from `css.hex`, `css.rgb`, `css.rgba`, and `css.named`; lengths from
`css.px`, `css.rem`, `css.em`, `css.pct`, `css.vh`, `css.vw`, and `css.auto`.
Because each property function only accepts the right *kind* of value — a length
where a length belongs, a color where a color belongs — a mismatched unit is a
type error rather than CSS that silently does nothing.

If you need a property the module doesn't have a named builder for, `css.property`
takes a raw name and value:

```pluma
css.property "text-transform" "uppercase"
```

## Inline styles

`css.inline` turns a list of declarations into a `style` string — useful for
one-off styling on a single element:

```pluma
use std/css

css.inline [css.color (css.hex "#e11"), css.padding (css.rem 1.0)]
# => "color: #e11; padding: 1rem"
```

## Reusable styles: rulesets

For a style you apply in more than one place — and for anything needing hover
states or media queries, which inline styles can't express — build a **ruleset**
with `css.rule`. A ruleset is the full power of CSS as a value:

```pluma
use std/css

def card :: css.ruleset = css.rule [
	css.padding (css.rem 1.0),
	css.border-radius (css.px 8.0),
	css.on "hover" [css.background (css.hex "#f3eefb")],
	css.media "(max-width: 600px)" [css.padding (css.rem 0.5)],
]
```

`css.on` adds a pseudo-class block (`"hover"`, `"focus"`), and `css.media` adds a
responsive block. To apply a ruleset, hand it to `view.styled`, which registers
its CSS once and gives the element a unique generated class — so styles are
scoped and never clash:

```pluma
use std/view

view.div [view.styled card] [view.text "A styled card"]
```

Since the property functions all share the `css` prefix, ruleset definitions read
more cleanly inside a [`using` block](/docs/reference/using):

```pluma
def card :: css.ruleset = using css {
	.rule [.padding (.rem 1.0), .border-radius (.px 8.0)]
}
```

Build bigger styles out of smaller ones with `css.compose`, which merges several
rulesets into one — a base style plus a variant, say. Later rulesets win on any
property they both set.

## Global styles

Most styling is scoped to an element, but a few rules belong to the document as a
whole — resetting margins, styling the `body`. `css.global` registers rules
against real selectors with `css.at`:

```pluma
use std/css

css.global [
	css.at "body" [css.margin (css.px 0.0), css.background (css.hex "#fff")],
]
```

## See also

- **[Views and HTML](/docs/stdlib/view)** — `view.styled` and `view.style-of`,
  which apply these styles to elements.
- **[Server-side rendering](/docs/deep-dives/ssr)** — how scoped styles are
  collected and emitted with the server-rendered HTML.
