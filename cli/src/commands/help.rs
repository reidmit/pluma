use compiler::{BINARY_NAME, LANGUAGE_NAME, VERSION};

pub(crate) fn print_help() {
	eprintln!(
		"{} v{}

Compiler & toolchain for the {} programming language

COMMANDS:
  [run] <path>     execute a module directly
  build <path> [--web] [-o out]
                   compile a module to a WasmGC deploy artifact (.wasm); run it
                   with `pluma run <out>.wasm`. `--web` builds a browser bundle
                   instead of a server/CLI artifact.
  dev <path> [--web] [--port N]
                   watch sources and reload on save. `--web` serves the browser
                   bundle with live-reload (default port 2222); otherwise (the
                   default) restarts the program on each change.
  format <path>... canonicalize formatting; pass `-` for stdin, `--check` to dry-run
  test [dir] [-f name]...
                   discover and run tests from `*.test.pa` files under the
                   nearest `pluma.pa`. Pass a directory to start the walk-up
                   from somewhere other than cwd. `-f name` (repeatable)
                   filters to modules whose name contains `name`.
  version          print compiler version info
  help             print this help text
",
		BINARY_NAME, VERSION, LANGUAGE_NAME
	);

	if cfg!(debug_assertions) {
		eprintln!(
			"DEBUG COMMANDS:
  analyze <path>   parse, type-check & dump info about a module
  tokenize <path>  dump the token stream for a module
",
		)
	}
}
