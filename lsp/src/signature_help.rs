use crate::completion::{
	KEYWORDS, is_ident_char, line_prefix, module_source, scan_uses, signature_of,
};
use crate::goto;
use compiler::ast::*;
use compiler::docs::doc_comment_for;
use compiler::{Diagnostic, Module};
use std::path::Path;

// Signature help for the call the cursor sits inside: the callee's one-line
// signature, the byte spans of each parameter within it, and which parameter
// is currently being typed. `lib.rs` maps this to the LSP `SignatureHelp`.
pub struct SigHelp {
	// The signature label, e.g. `def map :: fun (list a) (fun a -> b) -> list b`.
	pub label: String,
	pub doc: Option<String>,
	// `(start, end)` char offsets into `label` for each parameter, in order.
	// Empty when the callee's type isn't a function (nothing to highlight).
	pub params: Vec<(u32, u32)>,
	// Index of the parameter currently being entered. May equal `params.len()`
	// when more arguments have been typed than the function takes; the client
	// clamps the highlight.
	pub active_param: u32,
}

// Signature help at (`line`, `character`), or `None` when the cursor isn't
// inside a call whose callee resolves to a known signature. Detection is purely
// lexical — Pluma's calls are paren-free (`add x y`), so we find the enclosing
// application's head and count the argument tokens before the cursor — so it
// works while the line is mid-edit and doesn't parse.
pub fn signature_help(source: &[u8], path: &Path, line: u32, character: u32) -> Option<SigHelp> {
	let prefix = line_prefix(source, line, character);
	let (callee, active_param) = find_call(&prefix)?;
	let (label, doc) = resolve_callee(source, path, &callee)?;
	let params = fun_param_spans(&label);
	Some(SigHelp {
		label,
		doc,
		params,
		active_param: active_param as u32,
	})
}

fn is_callee_char(c: char) -> bool {
	is_ident_char(c) || c == '.'
}

// Operator / delimiter characters that separate one application from the next
// at the top grouping level. `-` is excluded (it's part of kebab-case
// identifiers like `flat-map`); `.` is excluded (qualified names, `list.map`);
// `"` stops a scan from reading code-like words out of string contents.
fn is_boundary(c: char) -> bool {
	matches!(
		c,
		'=' | '+' | '*' | '/' | '%' | '<' | '>' | '!' | '&' | '|' | '^' | '?' | ':' | '~' | ',' | '"'
	)
}

// The index in `chars` where the innermost application enclosing the cursor
// begins: scan left from the end, skipping balanced `()`/`[]`/`{}` groups (so a
// completed argument like `(g x)` is passed over whole), and stop just after the
// first boundary char or open bracket seen at depth 0.
fn find_app_start(chars: &[char]) -> usize {
	let mut depth = 0i32;
	let mut i = chars.len();
	while i > 0 {
		let c = chars[i - 1];
		match c {
			')' | ']' | '}' => {
				depth += 1;
				i -= 1;
			}
			'(' | '[' | '{' => {
				if depth == 0 {
					return i; // application starts just inside this opener
				}
				depth -= 1;
				i -= 1;
			}
			c if depth == 0 && is_boundary(c) => return i,
			_ => i -= 1,
		}
	}
	0
}

// The number of arguments fully entered before the cursor in `post` (the text
// after the callee). An argument is a maximal non-whitespace run at depth 0;
// it counts once a separating whitespace follows it. So `5 ` is one complete
// argument (active moves to the next), while `5` mid-token is still the active
// (first) argument.
fn count_complete_args(post: &[char]) -> usize {
	let mut depth = 0i32;
	let mut in_arg = false;
	let mut complete = 0;
	for &c in post {
		match c {
			'(' | '[' | '{' => {
				depth += 1;
				in_arg = true;
			}
			')' | ']' | '}' => {
				if depth > 0 {
					depth -= 1;
				}
			}
			c if c.is_whitespace() && depth == 0 => {
				if in_arg {
					complete += 1;
					in_arg = false;
				}
			}
			_ => in_arg = true,
		}
	}
	complete
}

// Resolve the enclosing call from the line prefix: the callee identifier and
// the active parameter index. `None` when the cursor isn't past a callee into
// its argument list.
fn find_call(prefix: &str) -> Option<(String, usize)> {
	let chars: Vec<char> = prefix.chars().collect();
	let start = find_app_start(&chars);

	let mut i = start;
	while i < chars.len() && chars[i].is_whitespace() {
		i += 1;
	}
	let callee = take_callee(&chars, &mut i)?;
	// A leading keyword introduces a sub-expression (`if foo x`, `while p`); the
	// real callee is the token after it.
	let callee = if KEYWORDS.contains(&callee.as_str()) {
		while i < chars.len() && chars[i].is_whitespace() {
			i += 1;
		}
		take_callee(&chars, &mut i)?
	} else {
		callee
	};

	// Require the cursor to be past the callee into argument position: there
	// must be whitespace separating the callee from what follows. Otherwise the
	// cursor is still on the callee name itself (completion's job, not ours).
	let post = &chars[i..];
	if post.first().is_none_or(|c| !c.is_whitespace()) {
		return None;
	}
	Some((callee, count_complete_args(post)))
}

// Consume a callee token (identifier chars plus `.` for qualified names like
// `list.map`) starting at `*i`, advancing `*i` past it. `None` if there's no
// identifier there.
fn take_callee(chars: &[char], i: &mut usize) -> Option<String> {
	let start = *i;
	while *i < chars.len() && is_callee_char(chars[*i]) {
		*i += 1;
	}
	(*i > start).then(|| chars[start..*i].iter().collect())
}

// Resolve a callee name to its signature label and doc. Qualified names
// (`list.map`) resolve through the imported module; bare names resolve against
// the current file's own top-level defs. Both slice the signature from source,
// so neither needs the (possibly mid-edit) current buffer to type-check.
fn resolve_callee(source: &[u8], path: &Path, callee: &str) -> Option<(String, Option<String>)> {
	if let Some((receiver, method)) = callee.split_once('.') {
		let full = scan_uses(source)
			.into_iter()
			.find(|(_, local)| local == receiver)
			.map(|(full, _)| full)?;
		let module = goto::imported_module(&full, path)?;
		let ast = module.ast.as_ref()?;
		let src = module_source(&module, &full);
		for def in &ast.body {
			if def.name.name != method {
				continue;
			}
			if !matches!(def.visibility, Visibility::Public | Visibility::Opaque) {
				continue;
			}
			let label = src.as_deref().and_then(|s| signature_of(s, def))?;
			let doc = doc_comment_for(&module, def.range.start.line);
			return Some((label, doc));
		}
		return None;
	}

	// Bare name: a top-level def in the file being edited.
	let mut module = Module::new("<lsp>".to_string(), std::path::PathBuf::new());
	let mut diagnostics: Vec<Diagnostic> = Vec::new();
	module.parse_from_bytes(source.to_vec(), &mut diagnostics);
	let ast = module.ast.as_ref()?;
	let text = String::from_utf8_lossy(source);
	for def in &ast.body {
		if def.name.name != callee {
			continue;
		}
		let label = signature_of(&text, def)?;
		let doc = doc_comment_for(&module, def.range.start.line);
		return Some((label, doc));
	}
	None
}

// The char-offset spans of each parameter in a signature label of the form
// `... :: fun P1 P2 -> R`. Parameters are the whitespace-separated type atoms
// (each possibly a parenthesized group) between `fun ` and the top-level `->`.
// Empty when the label has no `fun ` head (a non-function value).
fn fun_param_spans(label: &str) -> Vec<(u32, u32)> {
	let chars: Vec<char> = label.chars().collect();
	let Some(mut i) = find_fun_keyword(&chars) else {
		return Vec::new();
	};

	let mut spans = Vec::new();
	let mut depth = 0i32;
	let mut atom_start: Option<usize> = None;
	while i < chars.len() {
		let c = chars[i];
		// A top-level `->` ends the parameter list.
		if depth == 0 && c == '-' && chars.get(i + 1) == Some(&'>') {
			break;
		}
		match c {
			'(' | '[' | '{' => {
				if atom_start.is_none() {
					atom_start = Some(i);
				}
				depth += 1;
			}
			')' | ']' | '}' => depth -= 1,
			c if c.is_whitespace() && depth == 0 => {
				if let Some(s) = atom_start.take() {
					spans.push((s as u32, i as u32));
				}
			}
			_ => {
				if atom_start.is_none() {
					atom_start = Some(i);
				}
			}
		}
		i += 1;
	}
	if let Some(s) = atom_start.take() {
		spans.push((s as u32, i as u32));
	}
	spans
}

// The index just past a standalone `fun ` keyword in `chars`, or `None`.
fn find_fun_keyword(chars: &[char]) -> Option<usize> {
	let mut i = 0;
	while i + 3 < chars.len() {
		if chars[i] == 'f'
			&& chars[i + 1] == 'u'
			&& chars[i + 2] == 'n'
			&& chars[i + 3].is_whitespace()
			&& (i == 0 || !is_ident_char(chars[i - 1]))
		{
			return Some(i + 4);
		}
		i += 1;
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;

	fn call(prefix: &str) -> Option<(String, usize)> {
		find_call(prefix)
	}

	#[test]
	fn finds_callee_and_active_param() {
		// Paren-free application: active param advances as args complete.
		assert_eq!(call("\tadd "), Some(("add".into(), 0)));
		assert_eq!(call("\tadd 5"), Some(("add".into(), 0)));
		assert_eq!(call("\tadd 5 "), Some(("add".into(), 1)));
		assert_eq!(call("\tadd 5 6"), Some(("add".into(), 1)));
		assert_eq!(call("\tadd 5 6 "), Some(("add".into(), 2)));
		// Qualified callee.
		assert_eq!(call("\tlist.map xs "), Some(("list.map".into(), 1)));
		// A parenthesized argument counts as one.
		assert_eq!(call("\tf (g x) "), Some(("f".into(), 1)));
		// Innermost call wins: inside the group, the head is `g`.
		assert_eq!(call("\tprint (fact "), Some(("fact".into(), 0)));
		// Past a binary operator, a fresh application begins.
		assert_eq!(call("\t2 + foo "), Some(("foo".into(), 0)));
		// A leading keyword isn't the callee.
		assert_eq!(call("\tif foo "), Some(("foo".into(), 0)));
	}

	#[test]
	fn no_call_when_on_callee_or_empty() {
		// Still typing the callee name — not yet in argument position.
		assert_eq!(call("\tadd"), None);
		assert_eq!(call("\t"), None);
		// After an operator with nothing following.
		assert_eq!(call("\t2 + "), None);
	}

	#[test]
	fn param_spans_split_on_top_level_atoms() {
		let label = "def map :: fun (list a) (fun a -> b) -> list b";
		let spans = fun_param_spans(label);
		assert_eq!(spans.len(), 2, "two params, got {:?}", spans);
		let p0 = &label[spans[0].0 as usize..spans[0].1 as usize];
		let p1 = &label[spans[1].0 as usize..spans[1].1 as usize];
		assert_eq!(p0, "(list a)");
		assert_eq!(p1, "(fun a -> b)");
	}

	#[test]
	fn param_spans_empty_for_non_function() {
		assert!(fun_param_spans("def n :: int").is_empty());
	}

	#[test]
	fn resolves_stdlib_module_call() {
		// `list.map xs ` — second argument position over the baked stdlib. The
		// cursor sits past the trailing space (col 13), so `xs` is a complete
		// first argument and the second is active.
		let src = "use std/list\n\ndef main = fun {\n\tlist.map xs \n}\n";
		let help = signature_help(src.as_bytes(), &PathBuf::from("/proj/main.pa"), 3, 13)
			.expect("expected signature help for list.map");
		assert!(
			help.label.contains("map") && help.label.contains("fun"),
			"unexpected label: {:?}",
			help.label
		);
		assert_eq!(help.active_param, 1, "second argument is active");
		assert!(!help.params.is_empty(), "map's params should be spanned");
	}

	#[test]
	fn resolves_local_def_call() {
		let src =
			"def greet :: fun string -> string = fun name { name }\n\ndef main = fun {\n\tgreet \n}\n";
		let help = signature_help(src.as_bytes(), &PathBuf::from("/proj/main.pa"), 3, 7)
			.expect("expected signature help for local greet");
		assert!(help.label.contains("greet"), "label: {:?}", help.label);
		assert_eq!(help.active_param, 0);
	}

	#[test]
	fn none_outside_a_call() {
		let src = "def main = fun {\n\tlet x = 1\n}\n";
		assert!(signature_help(src.as_bytes(), &PathBuf::from("/proj/main.pa"), 1, 6).is_none());
	}
}
