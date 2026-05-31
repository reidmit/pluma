// The interactive REPL (`pluma repl`).
//
// Model: a REPL session is a growing Pluma program. Each submission is
// classified and routed into one of three buckets — import (`use ...`),
// top-level definition (`def`/`enum`/`alias`/`trait`/…), or a body line
// (a `let`/`while`/`defer` statement, or a bare expression). Every turn we
// re-render the whole session into one synthetic module:
//
//     <uses...>
//     <defs...>
//     def main = fun { <body lines...> }
//
// compile it from scratch, and run it in a fresh VM. Because the program is
// replayed in full each turn, scope/types/imports all "just work" through the
// normal pipeline with no incremental-compilation machinery. The two
// consequences we manage explicitly:
//
//   * Re-running replays every prior `print`, so we capture stdout into a
//     buffer and only surface the suffix produced since the last commit.
//   * A submission is only committed to the session if it compiles *and*
//     runs cleanly — a failing line leaves the session untouched.
//
// A bare expression additionally has its value echoed: we wrap the final
// expression as `{ __repl_value: (expr) }` so `main` returns a record we can
// read the value back out of (sidestepping the `err`-result exit handling in
// `VM::run`, and letting us suppress the echo when the value is `nothing`).

use crate::colors;
use crate::printing::print_diagnostics;
use compiler::{Compiler, Diagnostic, LANGUAGE_NAME, Token, Tokenizer, VERSION, find_project_root};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

// The synthetic module name the session is assembled into. Deliberately
// unlikely to collide with a real user module so `use my.module` still
// resolves against the project root.
const REPL_MODULE: &str = "__repl__";
// The record field a bare expression's value is parked in so we can read it
// back off `main`'s return value.
const ECHO_FIELD: &str = "__repl_value";

const PROMPT: &str = "pluma> ";
const CONT_PROMPT: &str = "   ... ";

// How a submitted line is routed into the session.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
	// `use ...` — an import line.
	Import,
	// A top-level definition (`def`/`enum`/`alias`/`trait`/`implement`,
	// optionally `public`/`opaque`). Added to the def list as if written at
	// the top level; a same-named definition replaces the previous one.
	Def,
	// A `let`/`while`/`defer`/`try` body statement — accumulated in `main`'s
	// body, no value echoed.
	Statement,
	// Anything else: a bare expression whose value we echo.
	Expr,
}

// The accumulated, known-good session.
struct Session {
	root_dir: PathBuf,
	uses: Vec<String>,
	defs: Vec<String>,
	// Body lines in submission order. Only `Statement`/`Expr` land here.
	body: Vec<(Kind, String)>,
	// Length of stdout produced by the committed session, so we can print only
	// the new suffix after each successful turn.
	committed_output_len: usize,
}

impl Session {
	fn new(root_dir: PathBuf) -> Self {
		Session {
			root_dir,
			uses: Vec::new(),
			defs: Vec::new(),
			body: Vec::new(),
			committed_output_len: 0,
		}
	}

	fn reset(&mut self) {
		self.uses.clear();
		self.defs.clear();
		self.body.clear();
		self.committed_output_len = 0;
	}
}

enum EvalError {
	// Genuine errors (warnings are filtered out upstream).
	Compile(Vec<Diagnostic>),
	Codegen(String),
	Runtime(vm::RuntimeError),
}

pub fn repl_command(_args: Vec<String>) {
	let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
	// Root the session at the nearest package marker so `use path.to.module`
	// resolves against the project, falling back to the working directory.
	let root_dir = find_project_root(&cwd).unwrap_or(cwd);
	let mut session = Session::new(root_dir.clone());

	println!(
		"{} v{} REPL  {}",
		colors::bold(LANGUAGE_NAME),
		VERSION,
		colors::dim(&format!("(scope: {})", root_dir.display()))
	);
	println!(
		"{}",
		colors::dim("Type an expression to evaluate it, or a `def`/`use` to extend the session.")
	);
	println!("{}", colors::dim("`:help` for commands · Ctrl-D to exit."));

	let mut rl = match DefaultEditor::new() {
		Ok(rl) => rl,
		Err(err) => {
			eprintln!("could not start the line editor: {}", err);
			return;
		}
	};

	let history = history_path();
	if let Some(path) = &history {
		let _ = rl.load_history(path);
	}

	loop {
		match read_input(&mut rl) {
			None => break,
			Some(input) => {
				let trimmed = input.trim();
				if trimmed.is_empty() {
					continue;
				}
				let _ = rl.add_history_entry(trimmed);

				if let Some(rest) = trimmed.strip_prefix(':') {
					if meta_command(rest, &mut session) {
						break;
					}
					continue;
				}

				handle_submission(&mut session, &input);
			}
		}
	}

	if let Some(path) = &history {
		let _ = rl.save_history(path);
	}
	println!();
}

// Run a `:meta` command. Returns true if the REPL should exit.
fn meta_command(rest: &str, session: &mut Session) -> bool {
	match rest.trim() {
		"help" | "h" | "?" => print_repl_help(),
		"quit" | "q" | "exit" => return true,
		"reset" => {
			session.reset();
			println!("{}", colors::dim("session reset"));
		}
		other => println!(
			"{}",
			colors::dim(&format!("unknown command `:{}` — try `:help`", other))
		),
	}
	false
}

fn print_repl_help() {
	println!("{}", colors::bold("REPL commands"));
	println!("  :help          show this help");
	println!("  :reset         clear all session bindings");
	println!("  :quit          leave the REPL (also Ctrl-D)");
	println!();
	println!("{}", colors::bold("Usage"));
	println!("  - Enter an expression to evaluate and print it:  1 + 2");
	println!("  - Bind with `let`:                                let x = 41");
	println!("  - Define top-level functions / types:            def double = fun n {{ n * 2 }}");
	println!("  - Import modules (stdlib or local):              use core.list");
	println!("  - Multi-line input continues while braces are open.");
}

// Read one logical submission, possibly spanning multiple physical lines while
// brackets remain open. Returns None on EOF (Ctrl-D).
fn read_input(rl: &mut DefaultEditor) -> Option<String> {
	let mut buffer = String::new();
	loop {
		let prompt = if buffer.is_empty() {
			PROMPT
		} else {
			CONT_PROMPT
		};
		match rl.readline(prompt) {
			Ok(line) => {
				if !buffer.is_empty() {
					buffer.push('\n');
				}
				buffer.push_str(&line);
				if is_complete(&buffer) {
					return Some(buffer);
				}
			}
			// Ctrl-C abandons the current (possibly multi-line) input and
			// returns to a fresh prompt — it does not exit the REPL.
			Err(ReadlineError::Interrupted) => return Some(String::new()),
			// Ctrl-D at the prompt exits.
			Err(ReadlineError::Eof) => return None,
			Err(err) => {
				eprintln!("input error: {}", err);
				return None;
			}
		}
	}
}

// A submission is complete once its brackets balance. Tokenizing (rather than
// scanning raw bytes) means braces inside strings/comments don't count.
fn is_complete(source: &str) -> bool {
	let bytes = source.as_bytes().to_vec();
	let mut depth: i64 = 0;
	for token in Tokenizer::from_source(&bytes) {
		match token {
			Token::LeftBrace(..) | Token::LeftParen(..) | Token::LeftBracket(..) => depth += 1,
			Token::RightBrace(..) | Token::RightParen(..) | Token::RightBracket(..) => depth -= 1,
			_ => {}
		}
	}
	depth <= 0
}

// Route the leading keyword of a submission to a session bucket.
fn classify(input: &str) -> Kind {
	let word: String = input
		.trim_start()
		.chars()
		.take_while(|c| c.is_ascii_alphabetic())
		.collect();
	match word.as_str() {
		"use" => Kind::Import,
		"public" | "opaque" | "def" | "enum" | "alias" | "trait" | "implement" => Kind::Def,
		"let" | "while" | "defer" | "try" => Kind::Statement,
		_ => Kind::Expr,
	}
}

// The name introduced by a definition, for replace-on-redefine. Handles a
// leading `public`/`opaque`, then `def`/`enum`/`alias`; `trait`/`implement`
// have no simple single name, so they're never deduped (None).
fn def_name(text: &str) -> Option<String> {
	let mut words = text.split_whitespace();
	let mut head = words.next()?;
	if head == "public" || head == "opaque" {
		head = words.next()?;
	}
	match head {
		"def" | "enum" | "alias" => words.next().map(|w| w.to_string()),
		_ => None,
	}
}

// The local name a `use` binds: the `as` alias if present, else the last
// dotted path segment (so `use util.mathx` binds `mathx`).
fn use_local_name(text: &str) -> Option<String> {
	let parts: Vec<&str> = text.split_whitespace().collect();
	if let Some(pos) = parts.iter().position(|&w| w == "as") {
		return parts.get(pos + 1).map(|s| s.to_string());
	}
	// parts[0] is `use`; parts[1] is the dotted path.
	parts
		.get(1)
		.and_then(|p| p.rsplit('.').next())
		.map(|s| s.to_string())
}

fn handle_submission(session: &mut Session, input: &str) {
	let kind = classify(input);
	let text = input.trim_end().to_string();

	// Build a trial copy of the session with the new submission applied. Only
	// committed on a clean compile + run.
	let mut uses = session.uses.clone();
	let mut defs = session.defs.clone();
	let mut body = session.body.clone();

	match kind {
		Kind::Import => {
			// Re-importing the same local name replaces the earlier binding, so a
			// `use core.list` followed by `use core.list as l` (or a corrected
			// path) doesn't trip the duplicate-import check.
			if let Some(name) = use_local_name(&text) {
				uses.retain(|u| use_local_name(u).as_deref() != Some(name.as_str()));
			}
			uses.push(text.clone());
		}
		Kind::Def => {
			if let Some(name) = def_name(&text) {
				defs.retain(|d| def_name(d).as_deref() != Some(name.as_str()));
			}
			defs.push(text.clone());
		}
		Kind::Statement | Kind::Expr => body.push((kind, text.clone())),
	}

	let source = render(&uses, &defs, &body);
	match evaluate(&session.root_dir, &source) {
		Ok((output, returned)) => {
			emit_new_output(session.committed_output_len, &output);
			if kind == Kind::Expr {
				echo_value(&returned);
			}
			// Commit.
			session.uses = uses;
			session.defs = defs;
			session.body = body;
			session.committed_output_len = output.len();
		}
		Err(err) => report(err),
	}
}

// Assemble the session into a single synthetic module source.
fn render(uses: &[String], defs: &[String], body: &[(Kind, String)]) -> String {
	let mut out = String::new();
	for u in uses {
		out.push_str(u);
		out.push('\n');
	}
	out.push('\n');
	for d in defs {
		out.push_str(d);
		out.push_str("\n\n");
	}

	out.push_str("def main = fun {\n");
	let last = body.len().saturating_sub(1);
	for (i, (kind, text)) in body.iter().enumerate() {
		match kind {
			// Park the final expression's value where we can read it back.
			Kind::Expr if i == last => out.push_str(&format!("{{ {}: ({}) }}\n", ECHO_FIELD, text)),
			// A non-final expression evaluates and is discarded; parenthesize so
			// it can't accidentally merge with the next line.
			Kind::Expr => out.push_str(&format!("({})\n", text)),
			// Statements (`let`/`while`/`defer`/`try`) are emitted verbatim —
			// wrapping them in parens would misparse.
			_ => {
				out.push_str(text);
				out.push('\n');
			}
		}
	}
	// When the body is empty or ends in a statement, give `main` a trailing
	// `nothing`-ish tail so it's a well-formed block that echoes nothing. An
	// empty block evaluates to an empty record, which carries no echo field.
	let needs_tail = body.last().map(|(k, _)| *k != Kind::Expr).unwrap_or(true);
	if needs_tail {
		out.push_str("{}\n");
	}
	out.push_str("}\n");
	out
}

// Compile the session source and run it, returning captured stdout and the
// value `main` returned.
fn evaluate(root_dir: &Path, source: &str) -> Result<(Vec<u8>, vm::Value), EvalError> {
	let mut compiler = Compiler::for_root_dir(root_dir.to_path_buf());
	compiler.set_module_source(REPL_MODULE.to_string(), source.as_bytes().to_vec());
	compiler.add_entry_module(REPL_MODULE.to_string());
	vm::stdlib::register_compiler(&mut compiler);

	if let Err(diagnostics) = compiler.check() {
		// `check()` surfaces warnings alongside errors. A growing REPL session
		// routinely has not-yet-used bindings, so proceed when only warnings
		// remain and let real errors stop the turn.
		let errors: Vec<Diagnostic> = diagnostics.into_iter().filter(|d| d.is_error()).collect();
		if !errors.is_empty() {
			return Err(EvalError::Compile(errors));
		}
	}

	let ir_program = ir::lower(&compiler).map_err(EvalError::Codegen)?;
	let program =
		codegen::compile_from_ir(&ir_program).map_err(|e| EvalError::Codegen(e.to_string()))?;

	let buffer = Rc::new(RefCell::new(Vec::<u8>::new()));
	let mut machine = vm::VM::new(program).with_stdout(vm::OutputSink::Buffer(Rc::clone(&buffer)));
	let returned = machine.run().map_err(EvalError::Runtime)?;
	let output = buffer.borrow().clone();
	Ok((output, returned))
}

// Print the slice of program output produced since the last commit.
fn emit_new_output(committed_len: usize, output: &[u8]) {
	let start = committed_len.min(output.len());
	let fresh = &output[start..];
	if fresh.is_empty() {
		return;
	}
	print!("{}", String::from_utf8_lossy(fresh));
	if !fresh.ends_with(b"\n") {
		println!();
	}
}

// Echo a bare expression's value, unless it was `nothing`.
fn echo_value(returned: &vm::Value) {
	if let vm::Value::Record(fields) = returned {
		if let Some(value) = fields.get(ECHO_FIELD) {
			if !matches!(value, vm::Value::Nothing) {
				println!("{}{}", colors::bold_green("→ "), value);
			}
		}
	}
}

fn report(err: EvalError) {
	match err {
		EvalError::Compile(diagnostics) => print_diagnostics(diagnostics),
		EvalError::Codegen(message) => eprintln!("{} {}", colors::bold_red("Error:"), message),
		EvalError::Runtime(error) => {
			if error.is_user_abort {
				eprintln!("{}", error.message);
			} else {
				eprintln!("{} {}", colors::bold_red("Runtime error:"), error.message);
			}
		}
	}
}

// `~/.pluma_history`, best-effort. None if we can't find a home directory.
fn history_path() -> Option<PathBuf> {
	std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pluma_history"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn classify_routes_by_leading_keyword() {
		assert!(matches!(classify("use core.list"), Kind::Import));
		assert!(matches!(classify("def f = fun x { x }"), Kind::Def));
		assert!(matches!(classify("public def f = 1"), Kind::Def));
		assert!(matches!(
			classify("opaque enum token { mk int }"),
			Kind::Def
		));
		assert!(matches!(classify("enum color { red green }"), Kind::Def));
		assert!(matches!(classify("let x = 5"), Kind::Statement));
		assert!(matches!(classify("while c { }"), Kind::Statement));
		assert!(matches!(classify("defer cleanup ()"), Kind::Statement));
		// `defer` must not be misread as the `def` prefix.
		assert!(matches!(classify("defer x"), Kind::Statement));
		assert!(matches!(classify("1 + 2"), Kind::Expr));
		assert!(matches!(classify("double 21"), Kind::Expr));
		// An identifier that merely starts with a keyword is an expression.
		assert!(matches!(classify("defx"), Kind::Expr));
		assert!(matches!(classify("-5"), Kind::Expr));
	}

	#[test]
	fn def_name_extracts_redefinable_name() {
		assert_eq!(def_name("def foo = fun x { x }").as_deref(), Some("foo"));
		assert_eq!(def_name("public def bar = 1").as_deref(), Some("bar"));
		assert_eq!(
			def_name("opaque enum baz { mk int }").as_deref(),
			Some("baz")
		);
		assert_eq!(def_name("alias id = int").as_deref(), Some("id"));
		// trait/implement have no single redefinable name.
		assert_eq!(def_name("trait show a { }"), None);
		assert_eq!(def_name("implement show for int { }"), None);
	}

	#[test]
	fn use_local_name_handles_alias_and_path() {
		assert_eq!(use_local_name("use core.list").as_deref(), Some("list"));
		assert_eq!(use_local_name("use util.mathx").as_deref(), Some("mathx"));
		assert_eq!(use_local_name("use core.list as l").as_deref(), Some("l"));
	}

	#[test]
	fn is_complete_tracks_bracket_depth() {
		assert!(is_complete("1 + 2"));
		assert!(is_complete("def f = fun x { x + 1 }"));
		assert!(!is_complete("def f = fun x {"));
		assert!(!is_complete("[1, 2,"));
		assert!(!is_complete("(1 +"));
		// Braces inside a string literal don't count as open brackets.
		assert!(is_complete("\"a { b\""));
	}

	#[test]
	fn render_wraps_only_the_final_expression() {
		let body = vec![
			(Kind::Statement, "let x = 5".to_string()),
			(Kind::Expr, "x + 1".to_string()),
		];
		let src = render(&[], &[], &body);
		assert!(src.contains("let x = 5\n"));
		assert!(src.contains(&format!("{{ {}: (x + 1) }}", ECHO_FIELD)));
		// A statement is never parenthesized.
		assert!(!src.contains("(let x = 5)"));
	}

	#[test]
	fn render_gives_a_statement_tailed_body_a_trailing_block() {
		let body = vec![(Kind::Statement, "let x = 5".to_string())];
		let src = render(&[], &[], &body);
		// Ends with an empty-block tail so `main` is a well-formed block.
		assert!(src.contains("let x = 5\n{}\n"));
	}

	#[test]
	fn render_handles_an_empty_body() {
		let src = render(&[], &[], &[]);
		assert!(src.contains("def main = fun {\n{}\n}"));
	}
}
