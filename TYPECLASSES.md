# TYPECLASSES.md

Design and implementation plan for adding typeclasses (a.k.a. "traits") to
Pluma. Captures the discussion from the design session — read top-to-bottom
to understand the system; jump to "Implementation phases" to execute.

## Why

Concrete capabilities that typeclasses unlock:

- **Numeric overloading.** Today `+`/`-`/`*`/`/` are hardcoded per-type in
  the analyzer; you can't write a generic `def square fun x { x * x }`
  that works on both int and float. This forces the `-float`-suffix
  workarounds we've explicitly been avoiding.
- **Generic sort.** `list.sort` needs a way to compare two values of
  unknown type. Today there's no way to express "any type that has a
  comparison."
- **Hash maps.** Generic maps need a hash function over arbitrary keys.
  Structural hash works at runtime but typeclasses let us forbid
  function-typed keys at compile time and let users provide custom hashes.

Things that we deliberately do NOT need typeclasses for:

- `==`/`!=` — already work structurally via `values_eq` for free.
- `to-string` — already works structurally for any value. Could become an
  `show` trait later if users want customization, but no v1 need.

## What we're shipping

### v1: `numeric` only

Vertical slice. Prove the entire machinery — declarations, instances,
constraint inference, dictionary codegen, dispatch — on the single
highest-visibility trait. Once this works end-to-end, the other traits are
templates.

### Followups (in this order)

- **`ord`** — `compare fun (a, a) -> ordering` where `ordering` is a
  prelude enum `lt | eq | gt`. Unblocks `list.sort` / `list.min` /
  `list.max`. Three instances: int, float, string.
- **`hash`** — `hash fun a -> int`. Unblocks generic `core.map`. Instances
  for int, float, string, bool, and prelude types.

### Explicitly deferred

- Multi-parameter typeclasses
- Functional dependencies
- `eq` and `show` as traits (structural already covers them)
- `mod` in `numeric` (defer to a future `integral` trait — only meaningful
  on ints)
- Numeric literal lifting / Haskell's `fromInteger`
- Haskell-style defaulting (Integer fallback for ambiguous constraints).
  We error instead and ask the user to annotate.

### Parametric instances are in scope

Originally deferred — promoted into the plan because deferring them
makes the trait system painful to use for anything generic. Without
parametric instances, `for ord on (list a) where (ord a)` is impossible;
users would have to write one instance per concrete list element type.
That's not viable for `core.list.sort` or `core.map`. See Phase 3 in
the implementation plan.

## Syntax

### Trait declaration

```pluma
def numeric trait a {
	add    fun (a, a) -> a
	sub    fun (a, a) -> a
	mul    fun (a, a) -> a
	div    fun (a, a) -> a
	negate fun a -> a
}
```

Fits the existing `def NAME KIND PARAMS BODY` shape (parallel to
`def color enum { ... }`, `def my-type alias other-type`). `a` is the
trait's type parameter.

### Default methods

Live in the trait body. Provide a fallback implementation that calls other
trait methods:

```pluma
def numeric trait a {
	add    fun (a, a) -> a
	negate fun a -> a
	sub    fun (a, a) -> a

	default sub fun (x, y) { add x (negate y) }
}
```

If an instance provides its own `sub`, that wins. Otherwise the default is
used.

### Instance declaration

```pluma
for numeric on int {
	def add    x y { int-add x y }
	def sub    x y { int-sub x y }
	def mul    x y { int-mul x y }
	def div    x y { int-div x y }
	def negate x   { int-sub 0 x }
}

for numeric on float {
	def add    x y { float-add x y }
	def sub    x y { float-sub x y }
	def mul    x y { float-mul x y }
	def div    x y { float-div x y }
	def negate x   { float-sub 0.0 x }
}
```

The `for T on U { ... }` block introduces a scope where:

- The trait `T` and type `U` are bound in context — no need to repeat them
  per method.
- Each method inside is a regular `def NAME fun ARGS { BODY }`. Bare names
  (no qualification) because the block scope tells the analyzer which
  trait+type this is for.

`int-add`, `float-add`, etc. are hidden VM builtins — they're the
primitives that the instances dispatch to. They're not exposed to users;
users always go through the trait.

### Parametric instances

For container types (`list a`, `option a`, etc.), the instance head is a
generic application, and the instance can require constraints on the
type parameters via a `where` clause:

```pluma
for showable on (option a) where (showable a) {
    def show o {
        when o is some v { "some $(showable.show v)" }
        is none { "none" }
    }
}

for showable on (list a) where (showable a) {
    def show xs {
        let parts = list.map xs fun x { showable.show x }
        "[$(string.join parts ", ")]"
    }
}
```

Multiple constraints are comma-separated inside the where clause:

```pluma
for showable on (pair a b) where (showable a, showable b) {
    def show p { "($(showable.show p.0), $(showable.show p.1))" }
}
```

The head's outer type constructor (`option`, `list`, `pair`) is what the
orphan rule pins on — the instance must live in the module that defines
either the trait or the head constructor. Type parameters (`a`, `b`)
don't count toward orphan eligibility — they're bound locally inside
the instance.

### Call-site syntax

Qualified at the call site, mirroring module-qualified calls. This is
intentional — it gives users one mental model ("qualified function call")
covering both module functions and trait methods, with the typeclass
dispatch hidden inside the analyzer.

```pluma
core.list.map xs fun-double      # module-qualified function
list.length xs                    # module-qualified function (short form)
numeric.add 1 2                   # trait method call — dispatch via int instance
ord.compare a b                   # trait method call — dispatch via instance of a's type
```

#### Disambiguation

The parser produces a `FieldAccess { head: numeric, field: add }` node
for `numeric.add` regardless of whether `numeric` is a module, trait, or
enum. The analyzer's resolution order is:

1. **Local variable** named `numeric` (rare — would have to be a param or
   let-binding).
2. **Module** named `numeric` (look up in the importer's `imports`).
3. **Trait** named `numeric` (look up in trait declarations).
4. **Enum** named `numeric` (variant constructor access — `numeric.foo`).

Each case produces a different resolved reference. The trait case adds
the class constraint `Class("numeric", α)` and emits an annotation on
the call site recording "this needs an instance dispatch."

**Collision rule**: a name that resolves to two of {module, trait,
enum} in the same scope is a declaration error caught at module load.
Namespaces compete for names — you can't have both `numeric` the module
and `numeric` the trait visible at once.

#### Operator desugaring

Operators are sugar over dotted trait calls. The parser keeps the
original `Operator::Add` AST node (so the LSP / formatter still see the
operator), but `constrain_expr` treats it as a typeclass method
reference for type inference, and codegen emits the dict-passing form.

| operator | desugars to | trait |
|---|---|---|
| `a + b` | `numeric.add a b` | `numeric` |
| `a - b` | `numeric.sub a b` | `numeric` |
| `a * b` | `numeric.mul a b` | `numeric` |
| `a / b` | `numeric.div a b` | `numeric` |
| `-a` (unary) | `numeric.negate a` | `numeric` |
| `a < b` | `ord.compare a b == lt` | `ord` (phase 4) |
| `a > b` | `ord.compare a b == gt` | `ord` (phase 4) |
| `a <= b` | `ord.compare a b != gt` | `ord` (phase 4) |
| `a >= b` | `ord.compare a b != lt` | `ord` (phase 4) |

Loading-order consequence: the prelude **must** define `numeric` and
its int/float instances before any user code uses `+`. Trivial in
practice (the prelude already has to be loaded first), but worth
knowing.

#### Higher-order use — passing a method as a value

A trait method can be passed without being called:

```pluma
list.fold xs 0 numeric.add
```

Here `numeric.add` is a value, type `Numeric α => (α, α) -> α`.
Combined with `list.fold : (list a, b, (b, a) -> b) -> b`, inference
unifies `a = b = α` and the constraint `Numeric α` flows to the outer
call site. If `xs : list int`, then `α = int`, the int instance is
selected, and `numeric.add` compiles to:

```
LoadGlobal(numeric_int_dict_slot)
GetDictField(0)              ; extract `add`
                             ; now a Value::Builtin(IntAdd) on the stack
```

That value is then passed straight to `fold`. **No special case needed
in `fold`** — the function it receives is just a regular 2-arg
callable.

The polymorphic-forwarding case works the same way:

```pluma
def fold-add fun xs init { list.fold xs init numeric.add }
; fold-add : Numeric a => list a -> a -> a
```

`fold-add` takes a hidden leading dict param; inside the body,
`numeric.add` compiles to "load my dict from the param, extract the
add field":

```
fn fold-add:
    LoadGlobal(list.fold)
    LoadLocal(1)             ; xs
    LoadLocal(2)             ; init
    LoadLocal(0)             ; my dict (forwarded from caller)
    GetDictField(0)          ; extract `add`
    Call(3)                  ; list.fold(xs, init, add)
```

#### Partial application

Pluma is uncurried — `numeric.add 5` would be an arity error, just like
calling any other 2-arg function with one arg. To partially apply, wrap
in a lambda (same convention as the rest of the language):

```pluma
let inc = fun y { numeric.add 1 y }
inc 5    # 6
```

The lambda captures both the dict (via the trait method reference) and
the first arg in the closure, then takes the remaining arg when called.
No new codegen story for partial application of trait methods — they
work exactly like partial application of any other function.

#### Known rough edges

1. **Error-message texture.** `core.list.foo` and `numeric.foo` both
   fail at the same AST node for different reasons (module has no
   `foo` vs trait has no `foo` vs ambiguous). The diagnostic phrasing
   has to look at *which* lookup attempt got closest and tailor the
   message accordingly.
2. **Visual collision with module-qualified calls.** A reader scanning
   `numeric.add 1 2` can't tell from syntax alone whether `numeric` is
   a module they imported or a trait. Behavior is the same (a function
   call), but for debugging it can be confusing — "why is `numeric`
   not found?" → because they didn't `use core.numeric` and it's
   actually a trait, not a module.
3. **Bare imports as future ergonomics.** Deferred: `use trait
   numeric.{add, sub}` to bring methods in unqualified. Adds a layer
   to identifier resolution. Revisit once we see real call-site
   ergonomics.

#### The design win

**One identifier-lookup mechanism covers locals, modules, traits, and
enums** — only the analyzer's interpretation of the resolved reference
differs. No new parser rule, no new operator, no new sigil. The parser
already produces the right AST shape for trait method calls; only the
analyzer's `FieldAccess` handler grows one new branch.

## Coherence

We use Rust's coherence model: orphan rule plus no-overlap. Strict but
mechanical — no runtime mystery.

### Orphan rule

An instance `for T on U` must be declared in the module that defines
either `T` or `U`. This makes coherence a syntactic property: you can't
write two conflicting instances from different modules.

### No overlap

When declaring a new instance, the head must not *unify* with any existing
instance of the same trait. With parametric instances in scope, this rule
does real work:

- `for showable on (list a)` plus `for showable on (list string)` →
  **conflict** (the heads unify with `a := string`).
- `for showable on int` plus `for showable on float` → fine (no
  unification possible — different head constructors).
- `for showable on (option a)` plus `for showable on (result a b)` →
  fine (different head constructors).

Implementation: at instance declaration, attempt to unify the new head
against every existing instance of the same trait. If any pair unifies,
reject with a "conflicts with existing instance at <location>"
diagnostic. The check is O(N_instances per trait); fine in practice.

Consequence: there's no "more specific" instance picking. If you want
specialized behavior for `list string`, you can't write it as a
specialization of `list a` — you'd have to use a separate trait or a
runtime branch inside the generic instance.

### Escape hatch

To add an instance for a type defined elsewhere (e.g. a stdlib type),
define a newtype alias and instance the alias:

```pluma
def my-int alias int

for my-trait on my-int { ... }
```

Mild verbosity, full coherence preserved.

### Prelude consequence

Prelude types (`option`, `result`) are owned by the prelude — instances on
them must live in the prelude. Third-party users can't add their own
instances to prelude types without newtyping. Acceptable.

## Type system: HM + class

We extend Hindley-Milner inference with class constraints (Wadler & Blott
1989).

### Mental model

The constraint set grows from "just type equations" to "type equations
plus class assertions like `Numeric α`." At any moment, each class
constraint is in one of three states:

1. **Concrete and dischargeable** (`Numeric int`) — look up the instance,
   drop the constraint.
2. **Concrete and undischargeable** (`Numeric color`) — type error.
3. **Still variable** (`Numeric α`) — keep it; either discharge later or
   push it into the function's generalized scheme.

### What changes vs the current pipeline

Today:

1. `constrain` produces `Vec<Constraint>` (type equations)
2. `unify` solves to a `Substitution`
3. `annotate` applies substitution back to AST

New:

1. `constrain` — also emits `Class { name, ty }` constraints when
   resolving trait methods
2. `unify` — equations as before; class constraints get rewritten by the
   substitution as it's built (no fixed-point solving needed)
3. **`discharge`** (new pass) — walks the class constraint set:
   - Concrete + instance exists → drop, record which instance was selected
     at this call site (for codegen).
   - Concrete + no instance → diagnostic.
   - Still variable → keep.
4. `annotate` — applies substitution + selected-instance info to AST

### Worked example

```pluma
def double fun x { numeric.add x x }
def main fun {
	print (double 5)        # int
	print (double 3.14)     # float
}
```

Inferring `double`:

1. Fresh `α` for `x`; scope: `x: α`
2. Look up `numeric.add` → instantiate `(α', α') -> α'` with constraint
   `{Numeric α'}`
3. Apply to `(x, x)`: unify `(α, α) -> β = (α', α') -> α'`; substitution
   `α' := α, β := α`; constraints rewritten to `{Numeric α}`
4. Function type: `α -> α`; constraints: `{Numeric α}`
5. Generalize at `def` boundary. `α` is free; quantify it; constraints
   involving `α` go into the scheme:
   `double : ∀α. Numeric α => α -> α`

Inferring `main`:

1. `double 5`: instantiate with fresh `β`, constraint `{Numeric β}`; unify
   `β := int`; constraint becomes `{Numeric int}`; instance exists →
   discharge.
2. `double 3.14`: same path, instance `Numeric float` → discharge.

### Implementation changes

```rust
enum Constraint {
	Eq(Type, Type),                       // existing
	Class { name: String, ty: Type },     // new
}

enum Scheme {
	// Was: Forall(Vec<usize>, Type)
	Forall(Vec<usize>, Vec<ClassConstraint>, Type)
}
```

At instantiation: generate fresh class constraints alongside fresh type
vars.

At generalization (def boundary):

1. Compute free type vars not bound in the surrounding env.
2. Partition class constraints: ones mentioning the free vars go into the
   scheme; ones over outer-env vars propagate outward.
3. Discharge any constraints that are now concrete; error if no instance.

### Ambiguity

After discharge, if a free type var has a class constraint but doesn't
appear in the def's signature (e.g. `show (read s)` — `read :: Read a =>
string -> a` and `show :: Show a => a -> string`, with `a` living only
between them), error and tell the user to add a type annotation. No
Haskell-style defaulting.

## Codegen: dictionary passing

A typeclass instance is **just a fixed-size record of method closures**,
threaded through calls as a hidden leading parameter. Everything else is
bookkeeping.

### Value::Dict

New value variant:

```rust
Value::Dict(Rc<Vec<Value>>)
```

A dictionary is a positional array of method values, indexed by trait
declaration order.

For `instance numeric int { add, sub, mul, div, negate }`, the dictionary
is:

```
Value::Dict([
	Value::Builtin(IntAdd),     // method index 0
	Value::Builtin(IntSub),     // method index 1
	Value::Builtin(IntMul),     // method index 2
	Value::Builtin(IntDiv),     // method index 3
	Value::Builtin(IntNegate),  // method index 4
])
```

### Instance compilation

Each `for T on U { ... }` instance produces one global slot at
program-load time, holding the dictionary `Value::Dict`. Method order in
the dictionary matches the trait's declaration order.

### Function compilation

Functions with K class constraints in their signature get K extra leading
parameters at the bytecode level. The user-facing arity is unchanged; the
codegen-emitted function takes `(dict₁, ..., dictₖ, user_arg₁, ...)`.

### Call sites

Each call to a trait method has been annotated by `discharge` with one of:

- `Resolved::Global(slot)` — concrete instance picked, load it from the
  named global slot.
- `Resolved::Forwarded(param_idx)` — the dict comes from the caller's own
  dict parameter; load it from a local slot.

A trait method call compiles to:

```
<load-dict-source>            # Global or Forwarded
GetDictField(method_idx)      # extract method (new instruction)
<load-args>
Call(arity)
```

### New VM instruction

```rust
GetDictField(u16)             // pop a Dict, push field at given index
```

### Closures over dicts

If a polymorphic function returns a lambda that uses trait methods, the
dict becomes a normal capture in the closure. The existing closure
machinery handles this transparently — no special case needed.

### Worked codegen

```pluma
def double fun x { numeric.add x x }
def main fun { print (double 5) }
```

```
; double — 2 params (dict at slot 0, x at slot 1)
fn double:
	LoadLocal(0)              ; the Numeric dict
	GetDictField(0)           ; extract method `add`
	LoadLocal(1)              ; x
	LoadLocal(1)              ; x
	Call(2)                   ; add(x, x)
	Return

; main
fn main:
	LoadGlobal(print_slot)
	LoadGlobal(double_slot)
	LoadGlobal(numeric_int_dict_slot)   ; picked by discharge
	LoadConst(5)
	Call(2)                   ; double(dict, 5) -> 10
	Call(1)                   ; print(10)
	Return
```

Polymorphic forwarding inside another constrained function:

```pluma
def quadruple fun x { double (double x) }       ; Numeric a => a -> a
```

```
fn quadruple:
	LoadLocal(0)              ; my dict
	LoadGlobal(double_slot)
	LoadLocal(0)              ; forward my dict to double
	LoadLocal(1)              ; x
	Call(2)                   ; double(dict, x)
	; ... result piped into another double call, same forwarding
```

### Parametric dispatch

Concrete instances (`for showable on int { ... }`) compile to a static
global Dict. Parametric instances (`for showable on (option a) where
(showable a) { ... }`) can't — the inner dict slot for `a` isn't known
until use-site. Each parametric instance compiles to **an instance
constructor function**: a function that takes the inner dicts as args
and returns a freshly-built Dict for the parameterized type.

Schematically, `for showable on (option a) where (showable a)` produces:

```
fn show_option_ctor(showable_a_dict):
    ; build the show method closure that captures the inner dict
    LoadDictConstructor                 ; build a Dict of size 1
    MakeClosure(show_option_body, [showable_a_dict])
    ; the closure body, when called with an option, runs the user's
    ; `when o is some v { ... }` code, calling showable.show recursively
    ; using showable_a_dict
    StoreDictField(0, <the closure>)
    ReturnDict
```

At a call site needing `Showable (Option Int)`, the compiler emits the
build-chain:

```
LoadGlobal(showable_int_dict_slot)        ; the inner dict
LoadGlobal(show_option_ctor_slot)         ; the parametric constructor
Call(1)                                   ; -> Dict for Showable (Option Int)
GetDictField(0)                           ; extract `show`
<load the option value>
Call(1)                                   ; show the option
```

Nested parametric chains (e.g., `Showable (List (Option Int))`) compose
naturally — build the inner dict first, then wrap it, then use the
result:

```
LoadGlobal(showable_int_dict_slot)
LoadGlobal(show_option_ctor_slot)
Call(1)                                   ; Dict for Showable (Option Int)
LoadGlobal(show_list_ctor_slot)
Call(1)                                   ; Dict for Showable (List (Option Int))
GetDictField(0)
<value>
Call(1)
```

**Caching is not done in v1.** Each call site that needs `Showable
(Option Int)` rebuilds the dict. Cost: a few extra allocations per
call. If profiling shows this matters, a discharge-time cache (memoize
on the type pattern) is the fix.

**Discharge phase changes**: when discharging a class constraint like
`Showable (Option α)`:
- If `α` is concrete → recursively discharge `Showable α`, record the
  whole chain (`InstanceChain { ctor: show_option, inner: [...] }`).
- If `α` is still a variable → keep the unsolved part; either it
  resolves later from another constraint, or it gets pushed into the
  enclosing scheme.

### Performance note

Each trait method call adds ~2 indirections (load dict, extract method)
over a normal call. Negligible in interpreted dispatch — the VM loop
dominates. If numeric hotspots ever matter, specialization at codegen
(monomorphize per concrete instantiation) is the knob to turn.
Parametric dispatch adds dict-construction overhead per call (see
above); cache at discharge time if needed.

## Implementation phases

### Phase 1 — Scaffolding (no trait usable yet)

- [x] Add `Constraint::Class { name, ty }` variant
- [x] Extend `Scheme::Forall` with class constraints
- [x] Add stubbed `discharge` pass between `unify` and `annotate`
- [x] Add `Value::Dict(Rc<Vec<Value>>)` to vm/src/value.rs
- [x] Add `GetDictField(u16)` to vm/src/instruction.rs (+ vm dispatch)
- [x] Parser: `def NAME trait PARAM { method-sigs+defaults }`
- [x] Parser: `for NAME on TYPE { defs }`
- [x] Parser: `default METHOD fun ARGS { BODY }` inside trait body
- [x] AST: `TraitNode`, `InstanceNode` definitions
- [x] AST annotation: `CallNode.resolved_instance: Option<Resolved>` for
      typeclass method calls

### Phase 2 — `numeric` end-to-end (vertical slice)

- [x] Add hidden VM builtins: `IntAdd`, `IntSub`, `IntMul`, `IntDiv`,
      `FloatAdd`, ..., `FloatNegate`
- [x] Declare `numeric` trait in prelude
- [x] Declare `for numeric on int { ... }` in prelude
- [x] Declare `for numeric on float { ... }` in prelude
- [x] Rewire `+`/`-`/`*`/`/`/unary `-` desugaring to `numeric.add` etc.
- [x] Constraint generation for `numeric.<method>` lookups
- [x] `discharge`: pick concrete instances when types are concrete
- [x] Generalization: push surviving constraints into schemes
- [x] Codegen: emit dict-passing form for constrained functions
- [x] Codegen: emit `LoadGlobal(instance_slot)` or `LoadLocal(forward_slot)`
      + `GetDictField(idx)` at each method call site
- [x] Test: fixture exercising `def double fun x { x + x }`
      polymorphically on int and float
- [x] Test: fixture demonstrating polymorphic forwarding
      (`def quadruple fun x { double (double x) }`)

### Phase 3 — Parametric instances

- [x] Parser: extend instance header to accept generic-application heads
      (`for T on (option a) { ... }`) — head is now a type expression,
      not just a name
- [x] Parser: `where (constraint, constraint, ...)` clause after the
      instance head
- [x] AST: `InstanceNode.head: TypeExpr`, `InstanceNode.constraints:
      Vec<ClassConstraint>`
- [x] Discharge: when matching a class constraint against an instance
      with a non-concrete head, unify head with constraint's type; on
      success, recursively add the instance's `where` constraints to
      the set being discharged
- [x] Annotation: replace single `Resolved::Global(slot)` with
      `Resolved::InstanceChain { ctor_slot, inner: Vec<Resolved> }` so
      nested parametric chains are representable
- [x] Codegen: emit an instance-constructor function per parametric
      instance — takes inner dicts as args, returns a freshly-built
      Value::Dict
- [x] Codegen: at call sites with InstanceChain, emit the chain of
      build calls (load inner, load ctor, Call, repeat)
- [ ] Overlap rule grows teeth: actually unify new head against all
      existing instances of the same trait, reject on unification
- [ ] Orphan rule keys on the head's outer type constructor
- [x] Test: fixture with `for showable on (option a) where (showable
      a)`, called with `option int`, `option string`, and nested
      `option (option int)`

**Also delivered as preconditions for Phase 3** (implicit in plan):

- [x] Register user-defined `def NAME trait` declarations in the
      analyzer (only the prelude `numeric` trait was registered in
      Phase 2)
- [x] Register user-defined `for T on U { ... }` instance declarations
      and compile their dictionaries
- [x] `MakeDict(N)` VM instruction for building a `Value::Dict` of
      closures at runtime (concrete + parametric)

### Phase 4 — Validation

- [x] Completeness: each instance must provide every trait method (or
      rely on a default)
- [x] Ambiguity detection in `discharge` (free var with constraint, not
      in signature)
- [x] Default method resolution: if instance omits a method, substitute
      the trait's default

### Phase 5 — `ord`

- [x] Declare `ordering` enum in prelude (`lt | eq | gt`)
- [x] Declare `ord` trait with `compare fun (a, a) -> ordering`
- [x] Instances for int, float, string
- [x] Parametric instance: `for ord on (option a) where (ord a)`,
      `for ord on (result a b) where (ord a, ord b)` (in baked-in
      `compiler/src/prelude.pa`). List awaits list patterns.
- [x] Rewire `<`/`>`/`<=`/`>=` to ord (via `compare`)
- [x] Add `core.list.sort` (takes a comparator returning `ordering`;
      pair with `ord.compare` for default sorting)

### Phase 6 — `hash`

- [x] Declare `hash` trait with `hash fun a -> int`
- [x] Instances for int, float, string, bool
- [x] Parametric instances: `for hash on (option a) where (hash a)`,
      `for hash on (result a b) where (hash a, hash b)` (in baked-in
      `compiler/src/prelude.pa`)
- [ ] Add `core.map` module (generic over hash + structural eq)

`core.map` is deferred — it'd be a fairly large stdlib module change
(buckets, resize, etc.) and isn't blocked on typeclass infrastructure
anymore. Land it when there's user demand.

## Open questions

These don't block phase 1 scaffolding but need answers before they get
hit:

1. **AST annotation for dispatch resolution.** Should
   `resolved_instance` live as a field on each call node, in a side table
   keyed on node id, or as a separate map? — Decide during phase 1.
2. **`default` keyword choice.** We used `default METHOD fun ... { BODY }`
   inside the trait body. Alternative: a trait method with a body is
   treated as a default (no `default` keyword); only signature-only ones
   are required. — Decide during phase 1 parser work.
3. **Bare imports of trait methods.** `use trait numeric.add as add`?
   Defer until we see real call-site ergonomics.
4. **Internal builtin naming.** We've been writing `int-add`, `float-add`
   for the hidden VM builtins. Should these be exposed (`int.add`,
   module-style) or stay hidden? Bias toward hidden — users always go
   through the trait. — Decide during phase 2.

---

## Appendix: Worked example — implementing `showable`

`showable` is not in the v1 shipping set (structural `to-string` already
covers its job), but walking through what implementing it *would* look
like is the cleanest way to see all the moving parts in action — built-in
instances, user enums and records, parametric instances, recursive use.
Treat this as design validation, not implementation work.

### A.1 — Trait declaration (lives in prelude)

```pluma
def showable trait a {
    show fun a -> string
}
```

One method, single param `a`, returns `string`.

### A.2 — Hidden VM builtins it needs

Each primitive instance needs a way to actually convert that primitive
to a string. Same pattern as `numeric`: add hidden VM builtins as the
primitives the instances dispatch to.

```rust
// in vm/src/builtin.rs
enum Builtin {
    ...
    IntToString,
    FloatToString,
    BoolToString,
}
```

Trivial eval impls: `format!("{}", n)` for int/float, `"true"`/`"false"`
for bool.

### A.3 — Instances for built-in primitives (in prelude)

All four live in the prelude because primitives don't belong to any
user module — the prelude owns them for orphan-rule purposes.

```pluma
for showable on int    { def show x { int-to-string x } }
for showable on float  { def show x { float-to-string x } }
for showable on bool   { def show x { bool-to-string x } }
for showable on string { def show x { x } }                  ; identity
```

The string instance is notable — `show` on a string returns the string
itself (no quotes added). Matches the existing `to-string "hello"`
which prints `hello`.

### A.4 — Parametric instances for prelude container types

With parametric instances in scope (per Phase 3), prelude container
types each get one instance covering all parameterizations:

```pluma
for showable on (option a) where (showable a) {
    def show o {
        when o is some v { "some $(showable.show v)" }
        is none { "none" }
    }
}

for showable on (result a b) where (showable a, showable b) {
    def show r {
        when r is ok v  { "ok $(showable.show v)" }
        is err e        { "err $(showable.show e)" }
    }
}

for showable on (list a) where (showable a) {
    def show xs {
        let parts = list.map xs fun x { showable.show x }
        "[$(string.join parts ", ")]"
    }
}
```

One declaration each. `showable (list (option int))` discharges by
chaining: first the int instance, then the option constructor (with
the int dict), then the list constructor (with the option-of-int
dict).

### A.5 — User-defined enum (no payload)

User types live in user modules, so the user writes the instance in
the same file (orphan rule).

```pluma
; user-module: colors.pa
def color enum { red; green; blue }

for showable on color {
    def show c {
        when c is red   { "red" }
        is green        { "green" }
        is blue         { "blue" }
    }
}
```

### A.6 — User-defined enum (with payload — recursive show)

```pluma
def shape enum {
    circle float
    square float
    rectangle float float
}

for showable on shape {
    def show s {
        when s is circle r {
            "circle(r=$(showable.show r))"
        } is square side {
            "square(side=$(showable.show side))"
        } is rectangle w h {
            "rectangle($(showable.show w) x $(showable.show h))"
        }
    }
}
```

Payload values get shown by calling `showable.show` recursively. Each
call introduces a constraint on the payload's type — but since `float`
already has an instance (from the prelude), the constraints discharge
at compile time and instance dispatch happens at runtime.

**Safety property**: if you tried to show an enum whose payload was a
function, you'd get a compile error: "no instance `showable` for `(int)
-> int`" — exactly the failure we want.

### A.7 — User-defined record

```pluma
def person alias { name: string, age: int }

for showable on person {
    def show p {
        "person(name=$(showable.show p.name), age=$(showable.show p.age))"
    }
}
```

(Note: `person` is an alias for a record shape. Worth confirming during
implementation whether instances can attach to aliases — probably yes
since the analyzer resolves the alias to a unique structural shape. If
not, the workaround is to make `person` a single-variant enum.)

### A.8 — Calling it at use sites

```pluma
print (showable.show 42)              ; "42"
print (showable.show 3.14)            ; "3.14"
print (showable.show true)            ; "true"
print (showable.show color.red)       ; "red"
print (showable.show (shape.circle 5.0))   ; "circle(r=5)"
print (showable.show (some 7))        ; "some 7"
print (showable.show [1, 2, 3])       ; "[1, 2, 3]"
print (showable.show [some 1, none])  ; "[some 1, none]"
```

In string interpolation:

```pluma
let user = { name: "alice", age: 30 }
print "user is $(showable.show user)"
; "user is person(name=alice, age=30)"
```

Passed as a value to a higher-order function:

```pluma
let formatted = list.map [1, 2, 3] showable.show
list.each formatted print
; "1"
; "2"
; "3"
```

`list.map xs showable.show` works because `showable.show` is a value
of type `Showable a => a -> string`. The constraint `Showable int`
gets fixed by the list's element type, instance discharges, and the
int's `show` method is what map actually receives.

### A.9 — Relationship to existing `to-string`

Today `to-string` is built-in, structural, and works on everything,
including types with no explicit instance. If we add `showable`:

- **Option A**: keep both. `to-string` stays structural ("stringify
  whatever this is"), `showable.show` is nominal ("use my type's
  defined instance"). Different semantics, both useful — `to-string`
  for quick debug printing, `showable.show` for customizable
  rendering. Many languages do this (Rust's `Debug` and `Display`).
- **Option B**: deprecate `to-string`, require every type have a
  `showable` instance. Cleaner but pushes work onto users.
- **Option C**: keep both initially; eventually make `to-string` an
  alias for `showable.show`.

**Recommendation: Option A indefinitely.** Structural debug-rendering
and nominal user-customizable formatting are genuinely different
flavors; keep both.

### A.10 — What this walkthrough validates about the design

- **Recursive instance calls just work.** Showing a `shape` calls
  `showable.show` on payload values, which dispatches via the payload
  type's instance. No special "auto-derive" mechanism — the user's
  instance body explicitly walks fields, recursing via `showable.show`.

- **Parametric instances make container support pleasant.** One
  instance per container, not per concrete instantiation. Justifies
  the work to move parametric out of "deferred."

- **The orphan rule plays out cleanly for user types.** User defines
  `color` in `colors.pa`, writes the instance in the same file —
  orphan-clean. Friction only appears when adding an instance for a
  type from another module — at which point the newtype-wrapper hop
  is appropriate.

- **The safety property holds.** "Show a function" is a compile-time
  error, not a runtime crash. Generic code that uses `showable.show
  x` requires `Showable α` in its signature, which can only discharge
  for types with instances.
