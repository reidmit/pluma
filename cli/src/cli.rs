//! The `pluma` command-line surface, defined declaratively with clap. Each
//! `Command` variant maps to one subcommand handler in `commands/`; `main` does
//! nothing but parse this and dispatch. Doc comments here ARE the `--help` text,
//! so keep them tight and user-facing.

use clap::{Parser, Subcommand};

/// Compiler & toolchain for the Pluma programming language.
#[derive(Parser)]
#[command(
	name = "pluma",
	version = compiler::VERSION,
	about,
	// `pluma` with no arguments prints help instead of erroring.
	arg_required_else_help = true
)]
pub(crate) struct Cli {
	#[command(subcommand)]
	pub command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
	/// Compile a module to WasmGC and run it on V8 (the deploy engine).
	///
	/// A `.pa` source is compiled fresh; a prebuilt `.wasm` runs directly.
	/// Everything after the path is passed to the program as its own argv.
	Run {
		/// Removed — `pluma run` always uses V8.
		#[arg(long, hide = true)]
		vm: bool,

		/// Module to run: a `.pa` source file or a prebuilt `.wasm` artifact.
		path: String,

		/// Arguments forwarded to the program (readable via `io.args`).
		#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
		program_args: Vec<String>,
	},

	/// Compile a module to a deploy artifact.
	///
	/// By default writes `<out>.wasm` (a server/CLI artifact); run it with
	/// `pluma run <out>.wasm`. `--web` instead writes a browser bundle. A
	/// fullstack directory (`server.pa` + `client.pa`) builds both halves.
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
