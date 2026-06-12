//! The `pluma` command-line surface, defined declaratively with clap. Each
//! `Command` variant maps to one subcommand handler in `commands/`; `main` does
//! nothing but parse this and dispatch. Doc comments here ARE the `--help` text,
//! so keep them tight and user-facing.

use clap::builder::Styles;
use clap::builder::styling::Style;
use clap::{Parser, Subcommand};

/// clap's default help styling, but with the underlines dropped from the section
/// headers and the usage line — they're `bold().underline()` by default; we keep
/// the bold and lose the underline. Every other role keeps its default.
const HELP_STYLES: Styles = Styles::styled()
	.header(Style::new().bold())
	.usage(Style::new().bold());

/// Compiler & toolchain for the Pluma programming language.
#[derive(Parser)]
#[command(
	name = "pluma",
	version = compiler::VERSION,
	about,
	styles = HELP_STYLES,
	// `pluma` with no arguments prints help instead of erroring.
	arg_required_else_help = true
)]
pub(crate) struct Cli {
	#[command(subcommand)]
	pub command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
	/// Compile and run a module or package.
	///
	/// A `.pa` source is compiled fresh and then run; a prebuilt `.wasm` runs
	/// directly.
	///
	/// Everything after the path is passed to the program as its own argv.
	Run {
		/// Enable `pluma dev` hot-reload rewrites (`signal.new` call-site keying).
		/// Hidden — a dev/diagnostic switch for exercising the hmr path headlessly.
		#[arg(long, hide = true)]
		hmr: bool,

		/// Module to run: a `.pa` source file or a prebuilt `.wasm` artifact.
		path: String,

		/// Arguments forwarded to the program (readable via `io.args`).
		#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
		program_args: Vec<String>,
	},

	/// Compile a module to a deploy artifact.
	///
	/// If given a `main.pa` file, generates a WASM file to be run with `pluma run <out>.wasm`.
	///
	/// If given a directory with a `client.pa` and `server.pa`, builds in "fullstack"
	/// mode. Generates `<out>.wasm` for the server, and a bundle of HTML/JS/WASM files for the client.
	///
	/// To build only a client bundle, do `pluma build --web path/to/client.pa`.
	Build {
		/// Build a browser bundle instead of a server/CLI artifact.
		#[arg(long)]
		web: bool,

		/// Output base name (or directory for a web/fullstack bundle).
		#[arg(short = 'o', long = "out", value_name = "OUT")]
		out: Option<String>,

		/// Base URL the generated RPC client targets (use "" for same-origin).
		#[arg(long = "server-url", value_name = "URL")]
		server_url: Option<String>,

		/// Binaryen wasm-opt level. Defaults to `3`; pass 2/3/4 for speed, s/z for
		/// size, or `0` to skip the pass entirely.
		#[arg(short = 'O', long = "optimize", value_name = "LEVEL", num_args = 0..=1, default_missing_value = "3")]
		optimize: Option<String>,

		/// Removed — use `--web` for a browser build, omit it otherwise.
		#[arg(long, hide = true, value_name = "TARGET")]
		target: Option<String>,

		/// Module to build: a `.pa` file or a fullstack directory.
		path: String,
	},

	/// Watch sources and reload on save.
	///
	/// Default mode restarts the program on each change. `--web` serves the
	/// browser bundle with live-reload over SSE.
	Dev {
		/// Serve the browser bundle with live-reload instead of restarting.
		#[arg(long)]
		web: bool,

		/// Port for the live-reload server (web/fullstack mode).
		#[arg(long, default_value_t = 2222, value_name = "PORT")]
		port: u16,

		/// Base URL the generated RPC client targets (fullstack mode).
		#[arg(long = "server-url", value_name = "URL")]
		server_url: Option<String>,

		/// Module to watch: a `.pa` file or a fullstack directory.
		path: String,
	},

	/// Canonicalize formatting in place.
	///
	/// Pass `-` to read a single module from stdin (writes to stdout).
	Format {
		/// Report files that would change and exit non-zero; don't rewrite them.
		#[arg(long)]
		check: bool,

		/// Files to format; `-` reads stdin.
		#[arg(value_name = "PATH")]
		paths: Vec<String>,
	},

	/// Report lint warnings for one or more modules.
	///
	/// Parses each file and flags stylistic and correctness smells (e.g. a
	/// `let _ = expr` that binds nothing). Exits non-zero if any lint fires, so
	/// it can gate CI.
	///
	/// With `--fix`, applies the autofixable lints in place instead of reporting
	/// them (a fixed file is reformatted). Pass `-` to read a single module from
	/// stdin (with `--fix`, the rewritten module is written to stdout).
	Lint {
		/// Apply autofixes in place instead of reporting.
		#[arg(long)]
		fix: bool,

		/// Files to lint; `-` reads stdin.
		#[arg(value_name = "PATH")]
		paths: Vec<String>,
	},

	/// Discover and run tests from `*.test.pa` files.
	///
	/// Walks up from the given directory (or cwd) to the nearest `pluma.pa`
	/// package root, then runs every suite it finds under V8.
	Test {
		/// Only run modules whose name contains this (repeatable).
		#[arg(short = 'f', value_name = "NAME")]
		filters: Vec<String>,

		/// Directory to start the walk-up from (default: current directory).
		dir: Option<String>,
	},

	/// Run the language server over stdio (spawned by editor extensions).
	LanguageServer,

	/// Print compiler version info.
	Version,

	/// Parse, type-check, and dump info about a module.
	#[cfg(debug_assertions)]
	Analyze {
		/// Module to analyze.
		path: String,
	},

	/// Dump the token stream for a module.
	#[cfg(debug_assertions)]
	Tokenize {
		/// Module to tokenize.
		path: String,
	},

	/// `pluma <path> [args…]` — shorthand for `pluma run <path> [args…]`.
	#[command(external_subcommand)]
	External(Vec<String>),
}
