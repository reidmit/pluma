# THENABLE.md

Design and implementation plan for the `try` chaining syntax — a
uniform way to sequence `result`, `option`, and (eventually) `task`
operations. Read top-to-bottom for the design; jump to "Implementation
phases" to execute.

Note on the filename: an earlier draft of this doc proposed a
`thenable` typeclass. That design is gone (see "What we considered"
below); the filename is historical. Consider renaming to `TRY.md` once
this lands.

## Why

Concrete capabilities this unlocks:

- **One sequential-bind syntax across carriers.** `result`, `option`,
  and (eventually) `task` all support the same `try x = expr` form.
  Without this we'd ship parallel constructs per type — `try` for
  result, `try?` for option, `await` for task — or nest `when`
  expressions until the indent walks off the right margin.
- **A type-checked foundation for async.** When `task a` arrives,
  adding it to `try` is one new `then` function and one new row in the
  dispatch table. The compiler doesn't need an `async`/`await`
  keyword pair; `try` carries the meaning.
- **An honest path through `result`/`option` chains today.** Even
  before async exists, the syntax pays off — fallible code that's
  currently three layers of `when` becomes straight-line.

Things we deliberately do NOT need:

- A `thenable` typeclass, higher-kinded types, or any HKT machinery.
  Each carrier gets its own `then` function; `try` is a syntactic form
  that dispatches by the head constructor of its RHS. See "What we
  considered" for why we rejected the typeclass route.
- A `?` postfix or `|?` pipe variant. `try` covers the use cases
  without inventing new operators. The pipe (`|`) and null-coalesce
  (`??`) stay as they are.
- A general "monad" abstraction. Each carrier is just itself; there's
  no shared interface beyond the convention of having a `then`
  function with a specific signature.

## What we considered

The first draft of this doc proposed `thenable` as a higher-kinded
typeclass, with `try` desugaring to `thenable.then` and dispatch
happening via class constraints. It would have let users make their
own types `try`-able and would have supported carrier-polymorphic
helpers.

We dropped it. The design cost — instance variables, slot conventions,
partial-application reasoning, an analyzer pass to discharge `IApp`
constraints, coherence questions, ambiguity diagnostics — wasn't
justified by the user-visible benefit. The Rust/Swift/F# approach (a
built-in `try` form that knows about a fixed set of carriers) is enough
for what we actually need, and the implementation is roughly 10% the
size.

What we give up:

- Users can't make their own types `try`-able. If they define
  `validation` or some custom error-collecting type, they can't `try`
  on it — they call its `then` (or write nested `when`s). In practice,
  most user error types wrap `result`, so this rarely comes up.
- No carrier-polymorphic helpers. A function that uses `try` internally
  is locked to one carrier. Write three versions if you need it for all
  three.
- The set of `try`-able carriers is fixed by the compiler. Each new one
  is a code change, not a library addition. For the foreseeable future
  the set is `{option, result, task}`.

We can always add HKT later if real use cases emerge. Crucially, doing
so wouldn't break user code — the surface syntax (`try`) is the same
either way.

## What we're shipping

### The three `then` functions

Each carrier exposes a `then` function in its own module. This requires
extracting `option` and `result` from `prelude.pa` into their own
module files (`option.pa`, `result.pa`), with the prelude
re-exporting the enum names. The existing parametric instances on
`option a` and `result a b` move with their enums. See Phase 1.

```pluma
; option.pa
def then fun (o, f) {
    when o is some v { f v }
    is none { none }
}

; result.pa
def then fun (r, f) {
    when r is ok v { f v }
    is err x { err x }
}

; task.pa (post-async)
def then fun (t, f) { task-bind t f }    ; native runtime hook
```

Type signatures:

```
option.then : (option a,   fun (a) -> option b)   -> option b
result.then : (result a e, fun (a) -> result b e) -> result b e
task.then   : (task a,     fun (a) -> task b)     -> task b
```

For `result.then`, both arms share the err type `e` — the continuation
must return a `result _ e` with the same `e` as the input. This is
enforced by the function's signature; no special machinery needed.

The three signatures are unrelated except by convention. There's no
shared trait.

Users can call `then` directly:

```pluma
let r = result.then (lookup-user id) fun u { ok u.name }
```

But the normal path is `try`.

### `try` syntax

A new binding form. `try x = expr` is `let`-shaped but desugars to a
`then` call that wraps the rest of the enclosing block. The analyzer
picks which `then` based on the inferred head constructor of `expr`.

```pluma
def get-user-and-profile fun id {
    try user = lookup-user id
    try profile = fetch-profile user.id
    ok (user, profile)
}
```

If `lookup-user : string -> result user err` and
`fetch-profile : string -> result profile err`, this desugars to:

```pluma
def get-user-and-profile fun id {
    result.then (lookup-user id) fun user {
        result.then (fetch-profile user.id) fun profile {
            ok (user, profile)
        }
    }
}
```

Mixing `try` with regular `let` and other expressions is fine — only
`try` lines trigger the wrap:

```pluma
def go fun id {
    try user = lookup-user id
    let greeting = "hello, $(user.name)"      ; plain let, inline
    print greeting                            ; expression, inline
    try _ = log "saw $(user.id)"              ; wildcard try is allowed
    ok user
}
```

Wildcard pattern (`try _ = ...`) is supported. Destructuring patterns
(`try (a, b) = ...`, `try {name: n} = ...`) work the same as in `let`
— the pattern binds in the `then` callback's parameter list.

### How dispatch works

The analyzer holds each `try` node in the AST until it has inferred the
type of the RHS. Then it looks at the type's head constructor:

| RHS head constructor | desugars to                                    |
|----------------------|------------------------------------------------|
| `option`             | `option.then expr fun pattern { rest }`        |
| `result`             | `result.then expr fun pattern { rest }`        |
| `task`               | `task.then expr fun pattern { rest }`          |
| anything else        | error: "`try` only works on option/result/task"|

The dispatch table lives in the analyzer. Adding a new carrier is a
small patch — add a row, ship a `then` function in the carrier's
module.

### Use sites

The three carriers chain identically:

```pluma
; result
def safe-divide-twice fun (a, b, c) {
    try q1 = safe-divide a b
    try q2 = safe-divide q1 c
    ok q2
}

; option
def first-positive fun (xs, ys) {
    try x = list.head xs
    try y = list.head ys
    if (x + y) > 0 is true { some (x + y) } else { none }
}

; task (post-async)
def fetch-page fun id {
    try user    = http.get "/users/$(id)"
    try profile = http.get "/users/$(id)/profile"
    task.return (user, profile)
}
```

Same syntax, three different `then` dispatches.

## Syntax — the new piece

### `try` parser shape

Production:

```
TryBinding ::= 'try' Pattern '=' Expr
```

Lives in the same place `let` does — inside a block, before the
remaining expressions of the block. Blocks keep `try` nodes
interspersed with other statements; the desugar walks top-down at
analysis time.

Disallowed: `try` at expression position (`let x = try ...` is a syntax
error). `try` is a *statement* form; it only makes sense in a block
context where there's tail to wrap.

## Type system: dispatch by inferred head

The only new machinery in the analyzer is **type-directed desugaring**
for `try`. Everything else is regular type checking.

### Order of operations

Normally desugar happens at parse time (e.g., operator-to-call
rewriting). For `try`, we can't desugar that early — we don't know
which carrier to dispatch to until the RHS has been type-checked.
Instead:

1. **Parse**: produce a `TryBinding` AST node verbatim.
2. **Constrain**: when the analyzer reaches a `TryBinding`, generate
   constraints for the RHS first.
3. **Peek + dispatch**: after applying the current substitution to the
   RHS's inferred type, read its head constructor. Rewrite the
   `TryBinding` in place to a `<module>.then` call wrapping the rest
   of the block as a continuation closure.
4. **Continue**: constrain the rewritten subtree normally.

The peek step is the only unusual part. In practice the RHS is almost
always concrete by the time we reach it (it came from a typed function
call, a literal, etc.). The implementation needs to be careful to
apply the substitution-so-far before reading the head — otherwise
free type variables make the head unreadable even when it would
become concrete shortly.

### When the RHS type isn't pinned

If the RHS's type is still a free type variable at the moment dispatch
happens, dispatch fails with a diagnostic:

> "`try`'s right-hand side has an undetermined type. Add a type
> annotation to its source."

Concrete example:

```pluma
def helper fun (f, x) {
    try y = f x          ; what's f's return type?
    ok y
}
```

Without an annotation on `f` or `helper`'s signature, `f x` has type
`?α` and the analyzer can't pick a carrier. The diagnostic points at
the `try` and suggests annotating `f`'s return type.

### Mixing carriers in one chain

Each `try` in a block dispatches independently. If one `try` resolves
to `result` and the next resolves to `option`, the desugared form has
nested `then` calls with incompatible signatures, and the regular type
checker fires on the inner call's expected continuation return:

```pluma
def broken fun id {
    try user = lookup-user id        ; result user err
    try name = list.head user.names  ; option string  -- BAD
    ok name
}
```

After dispatch, this desugars to:

```pluma
result.then (lookup-user id) fun user {
    option.then (list.head user.names) fun name {     ; inner
        ok name
    }
}
```

The outer `result.then`'s continuation must return `result _ err`. The
inner expression returns `option string`. Unification fails at the
inner expression: "expected `result _ _`, found `option string`."

The diagnostic should point at the second `try`, not the buried inner
call. Worth a polish pass to extract that.

### Same constructor, different captures

```pluma
def chain fun (x, y) {
    try a = x                  ; x : result int string
    try b = y                  ; y : result int bool
    ok (a, b)
}
```

Both `try`s dispatch to `result.then`. `result.then`'s signature
requires both arms to share the err type. The outer `result.then` is
instantiated with `e := string`; its continuation must return
`result _ string`. The inner `try`'s desugared form returns
`result _ bool`. Unification fails at the inner expression.

Same diagnostic shape as cross-carrier mixing; both types render as
`result` but the err types differ. Worth a tailored message: "this
`try` returns `result int bool`, but earlier `try`s in this chain
returned `result _ string`; err types must match across a `try` chain."

## Codegen: nothing new

`then` is a regular function. `try` desugars to a regular function
call at analyze time. Codegen sees only the desugared form. No new
bytecode, no new dispatch path, no dict-passing — none of it.

## Implementation phases

### Phase 1 — Extract carriers and add `then`

- [ ] Move `option` enum + its parametric instances (showable, ord,
      hash) from `prelude.pa` into `option.pa`
- [ ] Move `result` enum + its parametric instances from `prelude.pa`
      into `result.pa`
- [ ] Prelude re-exports `option` and `result` so existing user code
      that writes the type names bare keeps working
- [ ] Add `option.then` and `result.then` per the signatures above
- [ ] Test fixtures: direct calls to `option.then` and `result.then`

### Phase 2 — `try` syntax and dispatch

- [ ] Parser: `TryBinding ::= 'try' Pattern '=' Expr` inside block
      bodies
- [ ] AST: `BlockItem::TryBinding { pattern, expr, range }`
- [ ] Analyzer: when constraining a block, encounter a `TryBinding`,
      constrain the RHS, apply current substitution, peek the inferred
      head constructor of the RHS, rewrite the binding to a
      `<module>.then` call wrapping the remaining block items
- [ ] Analyzer: diagnostic when the RHS type isn't pinned by the time
      dispatch happens
- [ ] Analyzer: diagnostic when the RHS type's head is something other
      than option/result/task
- [ ] Test fixtures: `try` on result, on option, mixing-carrier error
      case, same-carrier-different-capture error case, nested `try`s

### Phase 3 — Polish

- [ ] Diagnostics: mixing-carrier error points at the conflicting `try`
      (not the inner desugared call)
- [ ] Diagnostics: same-carrier-different-capture error points at the
      conflicting `try` and explains the shared-capture requirement
- [ ] Diagnostics: undetermined-RHS error suggests an annotation site
- [ ] LSP: hover on `try` shows the inferred carrier
      (`option` / `result` / `task`)
- [ ] Formatter: `try` formatted like `let`

### Phase 4 — `task` (post-async)

- [ ] Once `task a` is implemented, add `task.then`
- [ ] Add a row to the dispatch table in the analyzer
- [ ] Test fixtures: `try` over tasks

## Open questions

1. **Top-level `try` ergonomics for scripts.**
   `def main = fun { try x = io.read-file "x" ; print x ; ok () }` —
   the trailing `ok ()` is mild noise for script-style code.
   Options:
   - Accept the noise; it's honest.
   - Allow `main` to return `nothing` and auto-discard the carrier's
     short-circuit at the top level (the runtime panics on `err`).
   - Provide a `run` helper in stdlib that takes a result-returning
     function and runs it.

   Decide during Phase 2.

2. **Should `list` get `then` too?**
   `list` has a natural concatMap-like `then`. Useful for
   list-comprehension-style code
   (`try x = xs; try y = ys; [(x, y)]`). But surprising to readers
   expecting fail-or-succeed semantics from `try`. Same syntax, very
   different meaning.

   **Lean: not in v1.** Adding it later is one row in the dispatch
   table and one function in `list`.

3. **Rename the file.**
   "Thenable" is a leftover from the typeclass design. Without the
   typeclass, there's no "thenable" concept — the functions are just
   called `then` and the syntax is `try`. Consider renaming this file
   to `TRY.md` when the design lands.

## Appendix: Worked walkthrough — the three carriers

This appendix exists for the same reason the showable walkthrough does
in TYPECLASSES.md: to validate that the design actually delivers the
"same syntax across carriers" promise by writing the same code three
times.

### B.1 — result

```pluma
def parse-user-from-line fun line {
    try parts = string.split line ","
    try name  = list.at parts 0      ; result string err
    try age-s = list.at parts 1
    try age   = int.parse age-s
    ok {name: name, age: age}
}
```

Each `try` short-circuits with `err _` on failure; the final `ok {...}`
produces the success value. The err type is shared across the chain
(enforced by `result.then`'s signature).

### B.2 — option

```pluma
def first-pair fun (xs, ys) {
    try x = list.head xs             ; option int
    try y = list.head ys
    some (x, y)
}
```

Each `try` short-circuits with `none` if either list is empty.

### B.3 — task (post-async)

```pluma
def fetch-dashboard fun user-id {
    try user    = http.get "/users/$(user-id)"          ; task user
    try posts   = http.get "/users/$(user-id)/posts"
    try friends = http.get "/users/$(user-id)/friends"
    task.return {user: user, posts: posts, friends: friends}
}
```

Same shape. Each `try` suspends until the task completes; if any task
errors or is cancelled, the chain short-circuits per `task.then`'s
definition (which the async runtime owns).

### B.4 — what the desugar actually produces

For B.2:

```pluma
def first-pair fun (xs, ys) {
    option.then (list.head xs) fun x {
        option.then (list.head ys) fun y {
            some (x, y)
        }
    }
}
```

For B.1, B.2, B.3 — the desugar shape is identical; the only thing
that changes is which module's `then` gets called. The analyzer picks
at dispatch time based on the RHS's inferred head.

### B.5 — mixing carriers (rejected)

```pluma
def broken fun id {
    try user = lookup-user id        ; result user err
    try name = list.head user.names  ; option string
    ok name
}
```

Compile error at the second `try`: the first `try` pinned the carrier
to `result`, but `list.head user.names : option string`. Diagnostic:
"this `try` returns `option string`, but earlier `try`s in this chain
returned `result _ _`; convert with `option-to-result` at the boundary
or split into separate functions."

### B.6 — what this validates

- **Same syntax, three carriers.** The user-facing code is identical
  across the three cases. The only difference is which module's `then`
  gets called; the dispatch is invisible.
- **Errors are local and honest.** Mixing carriers fails at the
  conflicting `try`, not at function exit.
- **No new operators or sigils.** `try` is the only new piece of
  surface syntax. The pipe, `??`, function-call, and field-access
  operators are all untouched.
- **No new type-system machinery.** No instance variables, no kind
  reasoning, no class constraints. Just regular function calls and
  regular type checking.
- **Future-proof for async.** B.3 is hypothetical today but textually
  identical to B.1 and B.2. When `task` lands, no surface-language
  work is needed — add `task.then`, add a dispatch row.
