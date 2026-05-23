# Packages

This doc describes how Pluma projects are structured, discovered, and configured. It captures decisions made during the initial design pass; details that are still open or deferred are flagged explicitly at the end.

## Motivation

Today, `pluma run foo.pa` improvises a "project root" from the entry file's directory, and `pluma test` improvises it from cwd. There is no stable, project-wide answer to "where does my project start." Symptoms:

- `pluma test` from above a project's directory mis-resolves `use` paths in test files
- No place to record dependencies, metadata, build settings, or workspace membership
- The LSP, formatter, and other tooling have to guess at scope

The fix is to introduce an explicit project marker file. Once we have one, the same mechanism unlocks dependencies, workspaces, and tooling that knows what "the project" means.

## The `pluma.pa` file

Every Pluma package has a marker file named `pluma.pa` at its root. It is a normal Pluma module — same grammar, same parser, same type system — with one special rule enforced by the analyzer: it must export a `def package` of type `core.package.info`.

```pluma
# pluma.pa
use core.package as p

def package :: p.info = {
	name: "my-app",
	version: "0.1.0",
}
```

### Why a `def` (not a new keyword)

Earlier drafts considered introducing a top-level `package "name" { … }` keyword that paralleled `test`. We rejected it: the wins were aesthetic (name in the syntactic head, distinct from a value def), but the costs were real (new tokenizer/parser/AST/analyzer surface, new errors, new ways for tools to break). A `def` reuses every existing piece of the language for free, and the only "magic" left is that one file by name is treated specially. We accept the resulting visual ambiguity with regular value defs as the price of structural simplicity.

### Evaluation: through the VM, not as static data

`pluma.pa` is compiled and executed like any module. The CLI forces the `package` global through the VM and reads the resulting record value. This is dramatically simpler than building a separate compile-time evaluator and unlocks ergonomic patterns like helper functions:

```pluma
use core.package as p

def github-repo :: fun string -> p.dep = fun url {
	p.git { url: url, branch: "main" }
}

def package :: p.info = {
	name: "my-app",
	version: "1.2.3",
	dependencies: [
		("bar", github-repo "github.com/x/y"),
	],
}
```

Tradeoffs accepted:
- A malformed config can produce a runtime error, not just a type error. The existing diagnostic rendering (`file:line:col`) handles this.
- Side effects (`print`, future I/O builtins) are technically reachable. Day-1 we trust users not to. We can sandbox later if needed.
- Infinite loops are technically possible. Same risk as any Pluma program. Timeout can be added if anyone hits it; probably nobody will.
- Tools that *write* `pluma.pa` (a hypothetical `pluma add foo`) become harder. Deferred until we have such a tool.

Each CLI invocation pays the cost of compiling + running `pluma.pa` (likely <10ms). Cacheable on mtime if it ever matters.

## Project root discovery

For any command that operates on a project, the runtime walks up from the starting directory until it finds a `pluma.pa`:

```
find_project_root(start):
  walk up from start:
    if pluma.pa exists in this dir → return this dir
  return None
```

| Command | Starting directory |
|---|---|
| `pluma run foo/bar.pa` | parent dir of `foo/bar.pa` |
| `pluma test [filter…]` | cwd |
| `pluma format <path>` | cwd |
| LSP / analyze | the file's directory |

The directory containing `pluma.pa` is the **package root** and the value of `Compiler.root_dir`. Every `use` path in that package resolves from there.

### No-marker behavior

Per command:

- **`pluma run path/to/foo.pa`** — falls back to the current rule (the entry file's directory becomes the root). Standalone scripts (`pluma run /tmp/foo.pa`) keep working without ceremony.
- **`pluma test`** — errors with `no package root found (no pluma.pa in any parent directory)` and runs no tests. Without a marker, there's no scope to enumerate; better to fail loudly than to guess at one.
- **LSP / analyze / format on a single file** — falls back to the file's directory as root. Useful when editing a one-off script.

### Multiple `pluma.pa` files in one repo

A repo may contain any number of `pluma.pa` files. Each one defines an independent package boundary. The walk-up finds the *nearest* one — never skips past it to find a higher one.

This is enough to support monorepos in phase 1: each subdirectory with its own `pluma.pa` is its own package, with its own dependencies, test scope, and identity. Coordination across them (a "workspace") is a phase-2 concept that does not affect this rule.

## Package boundaries

A package's `use` paths resolve from its package root. Code in `<root>/util/helpers.pa` and code in `<root>/main.pa` both write `use util.helpers` for the same import — paths are absolute from the package root, never sibling-relative.

Crossing a package boundary requires declaring the other package as a dependency. Code in package `bar` cannot just `use foo` if `foo` is a sibling package; it must declare `("foo", p.path "../foo")` (or, in phase 2, `p.workspace "foo"`) in its `pluma.pa` first.

Stdlib (`core.*`) is unaffected — those modules are baked into the compiler and visible to every package without ceremony.

### `pluma.pa` is not importable

The marker file's own module — named `pluma` after its file stem — is config, not code. Other modules in the package cannot `use pluma`. The analyzer rejects the import with a clear diagnostic, mirroring the rule that non-test modules cannot `use` a `.test` module. The intent is to keep project metadata one-directional: the CLI reads `pluma.pa`, but runtime code never depends on it.

## Repo structures

### Single package

```
my-app/
├── pluma.pa
├── main.pa
└── util/
    └── helpers.pa
```

One `pluma.pa` at the root. Most projects look like this.

### Library with multiple entry points

```
my-lib/
├── pluma.pa
├── lib.pa
├── lib.test.pa
└── examples/
    ├── basic.pa
    └── advanced.pa
```

Still one package (same name, version, deps). Multiple executables declared via a `binaries` field on the `package` info (added when needed; not in phase 1):

```pluma
def package :: p.info = {
	name: "my-lib",
	version: "1.0.0",
	binaries: [
		{ name: "basic", entry: "examples.basic" },
		{ name: "advanced", entry: "examples.advanced" },
	],
}
```

### Monorepo

```
monorepo/
├── pluma.pa              # optional: workspace metadata (phase 2)
└── packages/
    ├── foo/
    │   ├── pluma.pa      # package "foo"
    │   └── lib.pa
    └── bar/
        ├── pluma.pa      # package "bar"
        └── main.pa
```

Phase 1: each member operates independently. `pluma test` from `packages/foo/` tests just `foo`. Dependencies across packages are declared with explicit relative paths.

Phase 2 (deferred): the repo-root `pluma.pa` gains a `workspace` field that lists members and shared deps. Workspace-aware commands (`pluma test --workspace`) operate across all members.

## `core.package` schema

A new stdlib module. Sketch of the initial surface:

```pluma
# core.package (sketch)

enum dep {
	simple(string),
	full({ version: string, features: list string }),
	git({ url: string, branch: string }),
	path(string),
}

alias info {
	name: string,
	version: string,
	# All fields below are optional once we settle defaults.
	authors: list string,
	description: string,
	license: string,
	repository: string,
	dependencies: list (string, dep),
}
```

The fields above are a phase-1 sketch. Adding fields later is fine; removing or renaming them is a breaking change, so we should be deliberate.

### Free-form key→value data

Pluma records have fixed schemas, so things like `scripts: { "test-watch": "...", docs: "..." }` cannot be expressed as a record. The two valid Pluma idioms:

- **List of pairs** (default): `list (string, V)` — terse, ordered, schema-friendly. Used for `dependencies`, `features`, `scripts`.
- **List of records**: `list { name: string, ... }` — when each entry has ≥3 fields and self-documentation matters (e.g. `binaries`).

The build system converts these to whatever in-memory representation it needs.

### Common-mistake diagnostics

People coming from Cargo/npm will try to write `dependencies: { foo: "1.0.0", bar: "2.0.0" }`. The analyzer error there ("expected list of (string, dep) pairs, got a record") needs to be clear, since it'll be the most common first-encounter friction.

## CLI behavior reference

| Command | Where the root comes from | Scope |
|---|---|---|
| `pluma run path/to/foo.pa` | walk up from `path/to/` | one entry module + its imports |
| `pluma run path/to/foo.pa` (no marker found) | use `path/to/` as root | fallback; standalone script |
| `pluma test [filter…]` | walk up from cwd | all `*.test.pa` under the package root, filtered |
| `pluma test --workspace` (phase 2) | walk up from cwd, then up to workspace root | every member's tests |
| `pluma format` (no args) | walk up from cwd | every `.pa` file in the package |
| `pluma format <file>` | walk up from the file's dir | that one file |
| `pluma format <dir>` | walk up from the dir | every `.pa` file in that subtree |
| LSP / `pluma analyze` | walk up from the file | one module |

`pluma test` *always* scans from the package root, not cwd. Running `pluma test` from `my-app/util/` still tests the whole package — the walk-up finds `my-app/pluma.pa`, scope is `my-app/`. Narrowing is done with filter args, not cwd.

## Phasing

### Phase 1 (this design)

- `pluma.pa` marker file
- `core.package` module with `info` alias and `dep` variant
- Walk-up project root discovery
- `pluma.pa` evaluated through the VM
- Each `pluma.pa` is an independent package; multiple in one repo are allowed but uncoordinated
- `name` and `version` fields required; all others optional or deferred

### Phase 2 (deferred, design TBD)

- `workspace` field on `info` with `members: list string` (paths) and shared `dependencies`
- `p.workspace "name"` dep specifier
- `--workspace` flag on relevant commands
- `binaries: list { name, entry }` for multi-executable packages
- `features` field with conditional-compilation semantics
- `scripts` field with command shortcuts
- `pluma.pa` evaluation result caching (mtime-keyed) if compile time becomes a problem
- Optional sandbox mode that disables I/O builtins during config eval
- Optional config-eval timeout

### Not currently planned

- Static-only ("`const fn`-style") evaluation of `pluma.pa`. Rejected in favor of full VM evaluation.
- A separate non-Pluma config format (TOML, YAML, JSON). Rejected in favor of self-hosting.
- Field-as-keyword forms (`package "name" { … }`). Rejected in favor of plain `def`.

## Decided after the initial draft

- **Field names in `core.package.info`.** Locked in as sketched above for phase 1. Adding fields later is fine; removing or renaming is breaking, so be deliberate.
- **No-marker behavior** — see [No-marker behavior](#no-marker-behavior) above. `pluma test` errors; `pluma run` and per-file commands fall back to the current rule.
- **Analyzer fixtures and other "looks like Pluma but isn't a package" files.** The analyze/run test harnesses (`tests/analyze.rs`, `tests/run.rs`) drive the compiler in-process and don't go through project discovery, so `tests/analyze/*/main.pa` is unaffected. No special exclusion mechanism needed.
- **`pluma format` scope** — see the CLI table above. Single file, directory, or whole package depending on the argument shape.

## Open questions

None right now — see "Phase 2 (deferred, design TBD)" above for what's intentionally not pinned down yet.
