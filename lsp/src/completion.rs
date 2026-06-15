use crate::goto::{self, SymKind};
use compiler::ast::*;
use compiler::docs::doc_comment_for;
use compiler::{Diagnostic, Module};
use std::path::Path;

// What kind of thing a completion offers — drives the icon the editor shows
// and lets the client group results. Kept neutral here; `lib.rs` maps it to
// the LSP `CompletionItemKind`.
#[derive(Clone, Copy)]
pub enum CompletionKind {
	Function,
	Value,
	EnumType,
	Trait,
	Alias,
	Variant,
	Module,
	Keyword,
}

pub struct Completion {
	pub label: String,
	pub kind: CompletionKind,
	// The type signature, shown dimmed beside the label (`fun (list a) -> int`).
	pub detail: Option<String>,
	pub doc: Option<String>,
}

// Pluma keywords offered in open position. Deliberately small: only the words
// that start or structure an expression/definition, not punctuation.
const KEYWORDS: &[&str] = &[
	"def", "let", "fun", "use", "public", "opaque", "enum", "alias", "trait", "instance", "if",
	"else", "when", "is", "while", "try", "defer", "scope", "as", "remote",
];

/// Completions at (`line`, `character`) in `source`. Detects member access
/// (`receiver.<here>`) lexically — so it works while the line is mid-edit and
/// doesn't parse — and otherwise offers in-scope names, imported modules, and
/// keywords.
pub fn complete(source: &[u8], path: &Path, line: u32, character: u32) -> Vec<Completion> {
	let prefix = line_prefix(source, line, character);
	match member_receiver(&prefix) {
		Some(receiver) => member_completions(source, path, &receiver),
		None => scope_completions(source, line, character),
	}
}

// The text of `line` up to `character` (a UTF-16 column, but Pluma's surface is
// ASCII outside string/comment bodies, so treating it as a byte/char count is
// exact for the identifiers we scan here).
fn line_prefix(source: &[u8], line: u32, character: u32) -> String {
	let text = String::from_utf8_lossy(source);
	let Some(line_text) = text.lines().nth(line as usize) else {
		return String::new();
	};
	line_text.chars().take(character as usize).collect()
}

fn is_ident_char(c: char) -> bool {
	c.is_alphanumeric() || c == '_' || c == '-'
}

// If the cursor sits in member position (`receiver.partial`), return the
// receiver name. The partial after the dot is left for the client to filter on.
// A leading-dot member with no receiver (`.variant`, implicit-member sugar) has
// no receiver to resolve, so it falls through to open completion.
fn member_receiver(prefix: &str) -> Option<String> {
	let chars: Vec<char> = prefix.chars().collect();
	// Skip back over the partial identifier being typed.
	let mut i = chars.len();
	while i > 0 && is_ident_char(chars[i - 1]) {
		i -= 1;
	}
	// A dot must sit immediately before the partial.
	if i == 0 || chars[i - 1] != '.' {
		return None;
	}
	// The receiver is the identifier ending just before that dot.
	let dot = i - 1;
	let mut start = dot;
	while start > 0 && is_ident_char(chars[start - 1]) {
		start -= 1;
	}
	if start == dot {
		return None; // leading dot, no receiver
	}
	Some(chars[start..dot].iter().collect())
}

// -- member completion ----------------------------------------------------

fn member_completions(source: &[u8], path: &Path, receiver: &str) -> Vec<Completion> {
	// Resolve the receiver as an imported module first (the common case:
	// `list.`, `string.`, a `use … as alias`). The `use` set is scanned
	// lexically so this works even when the current line doesn't parse.
	if let Some(full_name) = scan_uses(source)
		.into_iter()
		.find(|(_, local)| local == receiver)
		.map(|(full, _)| full)
	{
		if let Some(module) = goto::imported_module(&full_name, path) {
			return module_member_completions(&module, &full_name);
		}
	}

	// Otherwise the receiver may be a locally-declared enum: `color.<variant>`.
	if let Some(variants) = local_enum_variants(source, receiver) {
		return variants;
	}

	Vec::new()
}

// Public top-level defs of an imported module, as completion items with their
// signature and doc. Private defs are skipped — they aren't reachable through
// the namespace.
fn module_member_completions(module: &Module, full_name: &str) -> Vec<Completion> {
	let Some(ast) = module.ast.as_ref() else {
		return Vec::new();
	};
	let source = module_source(module, full_name);

	let mut out = Vec::new();
	for def in &ast.body {
		if !matches!(def.visibility, Visibility::Public | Visibility::Opaque) {
			continue;
		}
		// Instances have no name to call through a namespace.
		if matches!(def.kind, DefinitionKind::Instance(_)) {
			continue;
		}
		let kind = def_kind(def);
		let detail = source.as_deref().and_then(|s| signature_of(s, def));
		let doc = doc_comment_for(module, def.range.start.line);
		out.push(Completion {
			label: def.name.name.clone(),
			kind,
			detail,
			doc,
		});
	}
	out
}

// Variants of a locally-declared `enum <receiver>`, for `receiver.<variant>`
// completion. The payload (if any) is rendered verbatim as the detail.
fn local_enum_variants(source: &[u8], receiver: &str) -> Option<Vec<Completion>> {
	let mut module = Module::new("<lsp>".to_string(), std::path::PathBuf::new());
	let mut diagnostics: Vec<Diagnostic> = Vec::new();
	module.parse_from_bytes(source.to_vec(), &mut diagnostics);
	let ast = module.ast.as_ref()?;

	for def in &ast.body {
		let DefinitionKind::Enum(en) = &def.kind else {
			continue;
		};
		if def.name.name != receiver {
			continue;
		}
		let items = en
			.variants
			.iter()
			.map(|v| Completion {
				label: v.name.name.clone(),
				kind: CompletionKind::Variant,
				detail: None,
				doc: doc_comment_for(&module, v.name.range.start.line),
			})
			.collect();
		return Some(items);
	}
	None
}

// -- open-position completion ---------------------------------------------

fn scope_completions(source: &[u8], line: u32, character: u32) -> Vec<Completion> {
	let mut out: Vec<Completion> = goto::visible_symbols(source, line, character)
		.into_iter()
		.map(|(name, kind)| Completion {
			label: name,
			kind: match kind {
				SymKind::Value => CompletionKind::Value,
				SymKind::Type => CompletionKind::Alias,
				SymKind::Namespace => CompletionKind::Module,
				SymKind::Variant => CompletionKind::Variant,
			},
			detail: None,
			doc: None,
		})
		.collect();

	for kw in KEYWORDS {
		out.push(Completion {
			label: (*kw).to_string(),
			kind: CompletionKind::Keyword,
			detail: None,
			doc: None,
		});
	}
	out
}

// -- helpers --------------------------------------------------------------

// The completion icon kind for a top-level def, from its syntactic shape.
// An `Expr` def whose body is a function literal is a Function; any other
// value-shaped def is a Value.
fn def_kind(def: &DefinitionNode) -> CompletionKind {
	match &def.kind {
		DefinitionKind::Expr(expr) => {
			if matches!(expr.kind, ExprKind::Fun(_)) {
				CompletionKind::Function
			} else {
				CompletionKind::Value
			}
		}
		DefinitionKind::Enum(_) => CompletionKind::EnumType,
		DefinitionKind::Trait(_) => CompletionKind::Trait,
		DefinitionKind::Alias(_) => CompletionKind::Alias,
		DefinitionKind::Instance(_) => CompletionKind::Value,
	}
}

// The one-line signature of a def, sliced from source: the header up to the
// def's `=` (value defs) or opening `{` (enum/trait), with the `public`/`opaque`
// keyword trimmed. e.g. `def map :: fun (list a) (fun a -> b) -> list b`.
fn signature_of(source: &str, def: &DefinitionNode) -> Option<String> {
	let line = source.lines().nth(def.range.start.line)?.trim();
	let cut = [line.find(" ="), line.find('{')]
		.into_iter()
		.flatten()
		.min()
		.unwrap_or(line.len());
	let header = line[..cut].trim();
	let header = header
		.strip_prefix("public ")
		.or_else(|| header.strip_prefix("opaque "))
		.unwrap_or(header)
		.trim();
	(!header.is_empty()).then(|| header.to_string())
}

// The source text of a loaded module: baked-in stdlib source by name, or the
// file on disk for a user module. Used to slice signatures (the `Module` itself
// doesn't retain its text).
fn module_source(module: &Module, full_name: &str) -> Option<String> {
	if let Some(src) = compiler::lookup_stdlib_source(full_name) {
		return Some(src.to_string());
	}
	std::fs::read_to_string(&module.module_path).ok()
}

// Lexically scan `use` lines for `(full_module_name, local_name)` pairs. Robust
// to an unparseable body (the common state mid-keystroke): imports sit at the
// top of the file and these lines parse on their own.
fn scan_uses(source: &[u8]) -> Vec<(String, String)> {
	let text = String::from_utf8_lossy(source);
	let mut out = Vec::new();
	for raw in text.lines() {
		let line = raw.trim();
		let Some(rest) = line.strip_prefix("use ") else {
			continue;
		};
		let mut parts = rest.split_whitespace();
		let Some(full) = parts.next() else {
			continue;
		};
		// `use a/b as c` → local `c`; otherwise the last path segment.
		let local = if parts.next() == Some("as") {
			match parts.next() {
				Some(alias) => alias.to_string(),
				None => continue,
			}
		} else {
			full.rsplit('/').next().unwrap_or(full).to_string()
		};
		out.push((full.to_string(), local));
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;

	fn labels(src: &str, line: u32, col: u32) -> Vec<String> {
		complete(src.as_bytes(), &PathBuf::from("/proj/main.pa"), line, col)
			.into_iter()
			.map(|c| c.label)
			.collect()
	}

	#[test]
	fn member_receiver_extraction() {
		assert_eq!(member_receiver("\tlet x = list."), Some("list".to_string()));
		assert_eq!(
			member_receiver("\tlet x = list.rev"),
			Some("list".to_string())
		);
		assert_eq!(member_receiver("\tlet x = flat-map"), None);
		assert_eq!(member_receiver("\tx."), Some("x".to_string()));
		// Leading dot (implicit-member sugar) has no receiver.
		assert_eq!(member_receiver("\t."), None);
	}

	#[test]
	fn completes_stdlib_module_members() {
		// `list.` offers list's public functions, with signatures.
		let src = "use std/list\n\ndef main = list.\n";
		let items = complete(src.as_bytes(), &PathBuf::from("/proj/main.pa"), 2, 16);
		let labels: Vec<&str> = items.iter().map(|c| c.label.as_str()).collect();
		assert!(labels.contains(&"map"), "expected map in {:?}", labels);
		assert!(labels.contains(&"reverse"), "expected reverse");
		// `map`'s detail carries its signature.
		let map = items.iter().find(|c| c.label == "map").unwrap();
		let detail = map.detail.as_deref().unwrap_or("");
		assert!(
			detail.contains("fun") && detail.contains("list"),
			"unexpected map detail: {:?}",
			detail
		);
	}

	#[test]
	fn respects_use_alias() {
		let src = "use std/list as l\n\ndef main = l.\n";
		let labels = labels(src, 2, 13);
		assert!(labels.contains(&"map".to_string()), "got {:?}", labels);
	}

	#[test]
	fn does_not_offer_private_defs() {
		// `std/list` has private helpers; completion must only surface `public`
		// names. We can't name a specific private def portably, so assert the
		// negative invariant via a local module instead.
		let dir = std::env::temp_dir().join(format!("pluma-cmpl-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("pluma.pa"), "").unwrap();
		std::fs::write(
			dir.join("helpers.pa"),
			"public def shown = fun { 1 }\ndef hidden = fun { 2 }\n",
		)
		.unwrap();
		let main = "use helpers\n\ndef main = helpers.\n";
		let main_path = dir.join("main.pa");
		let items = complete(main.as_bytes(), &main_path, 2, 19);
		let labels: Vec<&str> = items.iter().map(|c| c.label.as_str()).collect();
		assert!(labels.contains(&"shown"), "expected shown in {:?}", labels);
		assert!(
			!labels.contains(&"hidden"),
			"leaked private def: {:?}",
			labels
		);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn completes_local_enum_variants() {
		let src = "enum color {\n\tred\n\tgreen\n}\n\ndef c = color.\n";
		let labels = labels(src, 5, 14);
		assert!(labels.contains(&"red".to_string()), "got {:?}", labels);
		assert!(labels.contains(&"green".to_string()), "got {:?}", labels);
	}

	#[test]
	fn open_position_offers_scope_and_keywords() {
		let src = "def helper = fun { 1 }\ndef main = fun {\n\tlet local = 1\n\t\n}\n";
		// On the blank line inside `main` (line 3), offer the locals + keywords.
		let labels = labels(src, 3, 1);
		assert!(labels.contains(&"helper".to_string()), "got {:?}", labels);
		assert!(labels.contains(&"local".to_string()), "got {:?}", labels);
		assert!(labels.contains(&"when".to_string()), "missing keyword when");
	}
}
