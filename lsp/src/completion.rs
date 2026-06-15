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
	Field,
	Module,
	Keyword,
}

pub struct Completion {
	pub label: String,
	pub kind: CompletionKind,
	// The type signature, shown dimmed beside the label (`fun (list a) -> int`).
	pub detail: Option<String>,
	pub doc: Option<String>,
	// What the client filters this item against, when it differs from `label`.
	// Module paths set this so a `/`-containing label still matches the typed
	// prefix (clients treat `/` as a word boundary otherwise).
	pub filter_text: Option<String>,
	// An explicit replacement: `(range, new_text)`. Module-path completion
	// replaces the whole path token (which spans a `/`) rather than just the
	// word under the cursor.
	pub edit: Option<(compiler::Range, String)>,
}

impl Completion {
	fn new(
		label: impl Into<String>,
		kind: CompletionKind,
		detail: Option<String>,
		doc: Option<String>,
	) -> Self {
		Completion {
			label: label.into(),
			kind,
			detail,
			doc,
			filter_text: None,
			edit: None,
		}
	}
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
	// `use std/…` paths have no `.` and aren't expressions, so check that first.
	if let Some(path_start) = use_path_start(&prefix) {
		return module_path_completions(path, line, path_start, character);
	}
	match member_receiver(&prefix) {
		Some(receiver) => member_completions(source, path, line, character, &receiver),
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

fn member_completions(
	source: &[u8],
	path: &Path,
	line: u32,
	character: u32,
	receiver: &str,
) -> Vec<Completion> {
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

	// Last: the receiver is a value. If its inferred type is a record, offer
	// the record's fields.
	record_field_completions(source, path, line, character)
}

// Fields of the receiver's record type at the cursor. The buffer is mid-edit
// (`rec.<here>` doesn't parse), so we drop the partial member access at the
// cursor — keeping the receiver and every *other* field access, which is how an
// open record's fields get pinned — then analyze and read the receiver's type.
fn record_field_completions(
	source: &[u8],
	path: &Path,
	line: u32,
	character: u32,
) -> Vec<Completion> {
	let prefix = line_prefix(source, line, character);
	let total = prefix.chars().count();
	let partial_len = prefix
		.chars()
		.rev()
		.take_while(|c| is_ident_char(*c))
		.count();
	// `dot_col` is the `.`; the receiver's last char sits just before it.
	let Some(dot_col) = total.checked_sub(partial_len + 1) else {
		return Vec::new();
	};

	let sanitized = remove_on_line(source, line as usize, dot_col, character as usize);
	let result = crate::analysis::analyze_document(path, sanitized.into_bytes());
	let Some(module) = result.module else {
		return Vec::new();
	};

	let index = crate::hover::build_index(&module);
	let probe_col = dot_col.saturating_sub(1) as u32;
	let Some(hit) = crate::hover::lookup(&index, line, probe_col) else {
		return Vec::new();
	};

	let compiler::types::Type::Record(fields, _) = &hit.ty else {
		return Vec::new();
	};
	fields
		.iter()
		.map(|(name, ty)| {
			Completion::new(
				name.clone(),
				CompletionKind::Field,
				Some(format!("{}", ty)),
				None,
			)
		})
		.collect()
}

// Rebuild `source` with the character range [`from`, `to`) deleted from `line`.
// Other lines are untouched, so positions before `from` (and on every other
// line) stay valid against the result.
fn remove_on_line(source: &[u8], line: usize, from: usize, to: usize) -> String {
	let text = String::from_utf8_lossy(source);
	let mut out = String::new();
	for (i, l) in text.lines().enumerate() {
		if i > 0 {
			out.push('\n');
		}
		if i == line {
			let chars: Vec<char> = l.chars().collect();
			let from = from.min(chars.len());
			let to = to.min(chars.len());
			out.extend(chars[..from].iter());
			out.extend(chars[to..].iter());
		} else {
			out.push_str(l);
		}
	}
	out
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
		out.push(Completion::new(def.name.name.clone(), kind, detail, doc));
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
			.map(|v| {
				Completion::new(
					v.name.name.clone(),
					CompletionKind::Variant,
					None,
					doc_comment_for(&module, v.name.range.start.line),
				)
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
		.map(|(name, kind)| {
			let kind = match kind {
				SymKind::Value => CompletionKind::Value,
				SymKind::Type => CompletionKind::Alias,
				SymKind::Namespace => CompletionKind::Module,
				SymKind::Variant => CompletionKind::Variant,
			};
			Completion::new(name, kind, None, None)
		})
		.collect();

	for kw in KEYWORDS {
		out.push(Completion::new(*kw, CompletionKind::Keyword, None, None));
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

// -- use-path completion --------------------------------------------------

// If the cursor sits in the path of a `use` line (`use std/lis|`, `use |`),
// return the column where the path starts. `None` once the line moves past the
// path into `as <alias>`, or when the line isn't a `use` at all.
fn use_path_start(prefix: &str) -> Option<usize> {
	let chars: Vec<char> = prefix.chars().collect();
	let s = chars.iter().position(|c| !c.is_whitespace())?;
	// `use` followed by whitespace (not `used`, not a bare `use`).
	if chars.get(s..s + 3)? != ['u', 's', 'e'] {
		return None;
	}
	if !chars.get(s + 3).is_some_and(|c| c.is_whitespace()) {
		return None;
	}
	let mut p0 = s + 3;
	while chars.get(p0).is_some_and(|c| c.is_whitespace()) {
		p0 += 1;
	}
	// The path-so-far must be only module-path characters: an embedded space
	// means we've reached `as`, where module names no longer apply.
	chars[p0..]
		.iter()
		.all(|c| is_ident_char(*c) || *c == '/')
		.then_some(p0)
}

// Every module name a `use` could name: the baked-in stdlib plus the project's
// own `.pa` modules. Each replaces the whole path token so a `/`-containing
// name inserts cleanly regardless of how the client tokenizes the line.
fn module_path_completions(
	current: &Path,
	line: u32,
	path_start: usize,
	cursor: u32,
) -> Vec<Completion> {
	let range = compiler::Range::within_line(line as usize, path_start, cursor as usize);

	let mut names: Vec<String> = compiler::stdlib_sources()
		.iter()
		.map(|(n, _)| (*n).to_string())
		.collect();
	names.extend(project_module_names(current));
	names.sort();
	names.dedup();

	names
		.into_iter()
		.map(|name| {
			let mut c = Completion::new(name.clone(), CompletionKind::Module, None, None);
			c.filter_text = Some(name.clone());
			c.edit = Some((range, name));
			c
		})
		.collect()
}

// User modules reachable in the current project: every non-test `.pa` file
// under the project root, as a slash-separated module name, excluding the file
// being edited. stdlib and build/VCS dirs are skipped.
fn project_module_names(current: &Path) -> Vec<String> {
	let Some(root) =
		compiler::find_project_root(current).or_else(|| current.parent().map(Path::to_path_buf))
	else {
		return Vec::new();
	};
	let current_module = module_name_of(&root, current);

	let mut out = Vec::new();
	let mut stack = vec![root.clone()];
	while let Some(dir) = stack.pop() {
		let Ok(entries) = std::fs::read_dir(&dir) else {
			continue;
		};
		for entry in entries.flatten() {
			let path = entry.path();
			let name = entry.file_name();
			let name = name.to_string_lossy();
			if path.is_dir() {
				if !name.starts_with('.') && name != "target" && name != "node_modules" {
					stack.push(path);
				}
			} else if name.ends_with(".pa")
				&& !name.ends_with(".test.pa")
				&& name != compiler::PROJECT_MARKER_FILE
			{
				if let Some(m) = module_name_of(&root, &path) {
					if Some(&m) != current_module.as_ref() {
						out.push(m);
					}
				}
			}
		}
	}
	out
}

// A file's module name relative to the project root: path separators become
// `/`, the `.pa` suffix is dropped (`src/util.pa` → `src/util`).
fn module_name_of(root: &Path, file: &Path) -> Option<String> {
	let rel = file.strip_prefix(root).ok()?;
	let s = rel.to_string_lossy().replace('\\', "/");
	Some(s.strip_suffix(".pa").unwrap_or(&s).to_string())
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
	fn use_path_context_detection() {
		assert_eq!(use_path_start("use "), Some(4));
		assert_eq!(use_path_start("use std"), Some(4));
		assert_eq!(use_path_start("use std/li"), Some(4));
		assert_eq!(use_path_start("use std/sys/"), Some(4));
		// Past the path, in alias position — no longer module names.
		assert_eq!(use_path_start("use std/list as "), None);
		// Not a use line.
		assert_eq!(use_path_start("def x = list"), None);
		assert_eq!(use_path_start("used"), None);
	}

	#[test]
	fn completes_stdlib_module_paths() {
		// `use std/` offers stdlib module names, each replacing the whole path.
		let src = "use std/\n";
		let items = complete(src.as_bytes(), &PathBuf::from("/proj/main.pa"), 0, 8);
		let labels: Vec<&str> = items.iter().map(|c| c.label.as_str()).collect();
		assert!(
			labels.contains(&"std/list"),
			"expected std/list in {:?}",
			labels
		);
		assert!(labels.contains(&"std/string"), "expected std/string");
		assert!(
			labels.contains(&"std/sys/process"),
			"expected nested module"
		);
		// The item replaces the path token (cols 4..8) with the full name, and
		// filters against the full path so `std/` still matches `std/list`.
		let list = items.iter().find(|c| c.label == "std/list").unwrap();
		assert_eq!(list.filter_text.as_deref(), Some("std/list"));
		let (range, new_text) = list.edit.as_ref().expect("expected a text edit");
		assert_eq!((range.start.col, range.end.col), (4, 8));
		assert_eq!(new_text, "std/list");
	}

	#[test]
	fn completes_user_module_paths() {
		let dir = std::env::temp_dir().join(format!("pluma-usepath-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(dir.join("util")).unwrap();
		std::fs::write(dir.join("pluma.pa"), "").unwrap();
		std::fs::write(dir.join("helpers.pa"), "public def a = 1\n").unwrap();
		std::fs::write(dir.join("util/math.pa"), "public def b = 1\n").unwrap();
		std::fs::write(dir.join("main.test.pa"), "def tests = []\n").unwrap();

		let main_path = dir.join("main.pa");
		let items = complete(b"use \n", &main_path, 0, 4);
		let labels: Vec<&str> = items.iter().map(|c| c.label.as_str()).collect();
		assert!(
			labels.contains(&"helpers"),
			"expected helpers in {:?}",
			labels
		);
		assert!(labels.contains(&"util/math"), "expected nested user module");
		// Test files and the project marker aren't importable modules.
		assert!(
			!labels.contains(&"main.test"),
			"leaked test module: {:?}",
			labels
		);
		assert!(!labels.contains(&"pluma"), "leaked project marker");
		// stdlib still offered alongside user modules.
		assert!(labels.contains(&"std/list"), "stdlib missing");
		let _ = std::fs::remove_dir_all(&dir);
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

	// A throwaway single-file project, returning the path to `main.pa`.
	fn temp_main(src: &str) -> (PathBuf, std::path::PathBuf) {
		let dir = std::env::temp_dir().join(format!("pluma-rec-{}-{}", std::process::id(), src.len()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("pluma.pa"), "").unwrap();
		let main = dir.join("main.pa");
		std::fs::write(&main, src).unwrap();
		(dir, main)
	}

	#[test]
	fn completes_record_fields_from_let_literal() {
		// `rec` is a closed record from its literal; `rec.` offers its fields.
		let src = "def main = fun {\n\tlet rec = { name: \"a\", age: 1 }\n\trec.\n}\n";
		let (dir, main) = temp_main(src);
		let items = complete(src.as_bytes(), &main, 2, 5);
		let labels: Vec<&str> = items.iter().map(|c| c.label.as_str()).collect();
		assert!(labels.contains(&"name"), "expected name in {:?}", labels);
		assert!(labels.contains(&"age"), "expected age in {:?}", labels);
		// The field's type rides along as the detail.
		let age = items.iter().find(|c| c.label == "age").unwrap();
		assert_eq!(age.detail.as_deref(), Some("int"));
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn completes_open_record_fields_pinned_by_other_access() {
		// `r`'s type is only known through field accesses. The one at the cursor
		// is dropped during completion, but `r.name` elsewhere still pins `name`.
		let src = "def f = fun r {\n\tlet n = r.name\n\tr.\n}\n";
		let (dir, main) = temp_main(src);
		let items = complete(src.as_bytes(), &main, 2, 3);
		let labels: Vec<&str> = items.iter().map(|c| c.label.as_str()).collect();
		assert!(labels.contains(&"name"), "expected name in {:?}", labels);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn no_fields_for_non_record_receiver() {
		// `n` is an int — no fields to offer (and no crash / no garbage items).
		let src = "def main = fun {\n\tlet n = 1\n\tn.\n}\n";
		let (dir, main) = temp_main(src);
		let items = complete(src.as_bytes(), &main, 2, 3);
		assert!(
			items.is_empty(),
			"expected no items, got {:?}",
			items.iter().map(|c| &c.label).collect::<Vec<_>>()
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
