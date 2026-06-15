//! Documentation extraction: turn an analyzed module into a structured doc
//! model (`ModuleDoc`), and render that model as a generated Pluma data module.
//!
//! The frontend already does the hard parts — parsing, type inference, and
//! associating a `#` comment block with the def below it — so a doc generator
//! is just a harvest: walk a module's public items, pair each with its rendered
//! type signature and its doc comment, and split the prose from the indented
//! example blocks. The LSP's hover reads the very same comment blocks, so the
//! association logic (`doc_comment_for`, `module_doc_comment`) lives here and is
//! shared by both.

use crate::ast::*;
use crate::highlight;
use crate::location::Range;
use crate::module::Module;

/// One classified slice of a code block. `class` is a highlight class name
/// (`keyword`, `string`, …) or `""` for plain text. Concatenating the `text`s
/// reproduces the block.
pub struct DocSpan {
	pub class: String,
	pub text: String,
}

/// One paragraph of a doc comment. `kind` is `"p"` for prose or `"code"` for an
/// example block. A prose block carries its `text`; a code block carries
/// pre-highlighted `spans` (its `text` is left empty). Splitting and
/// highlighting happen once here so every renderer gets the same structure.
pub struct DocBlock {
	pub kind: &'static str,
	pub text: String,
	pub spans: Vec<DocSpan>,
}

// A prose paragraph (rendered as inline markdown by the consumer).
fn prose_block(text: String) -> DocBlock {
	DocBlock {
		kind: "p",
		text,
		spans: Vec::new(),
	}
}

// A code block, syntax-highlighted now via the shared classifier so the docs and
// the editor agree. The raw text moves into `spans`, so `text` is left empty.
fn code_block(text: String) -> DocBlock {
	let spans = highlight::spans(&text)
		.into_iter()
		.map(|s| DocSpan {
			class: s.class.map(|c| c.name().to_string()).unwrap_or_default(),
			text: s.text,
		})
		.collect();
	DocBlock {
		kind: "code",
		text: String::new(),
		spans,
	}
}

/// A single documented top-level item.
pub struct ItemDoc {
	pub name: String,
	/// The rendered type signature, e.g. `fun (list a) -> int`, or a header like
	/// `enum option a` for a type definition.
	pub signature: String,
	pub blocks: Vec<DocBlock>,
}

/// Everything we document about one module.
pub struct ModuleDoc {
	/// The full module path, e.g. `std/list`.
	pub name: String,
	/// The namespace a `use` binds it to — the last path segment (`list`).
	pub namespace: String,
	pub blocks: Vec<DocBlock>,
	pub items: Vec<ItemDoc>,
}

/// Harvest the doc model from an analyzed module. `module.ast` must be present
/// and carry inferred types (i.e. the module went through `check()`), since
/// value-def signatures come from each def's inferred `Type`. `source` is the
/// module's original text, used to render type/trait/alias declarations
/// verbatim (those have no `Type` to print).
pub fn extract(module: &Module, module_name: &str, source: &str) -> ModuleDoc {
	let namespace = module_name
		.rsplit('/')
		.next()
		.unwrap_or(module_name)
		.to_string();

	let mut items = Vec::new();
	if let Some(ast) = module.ast.as_ref() {
		for def in &ast.body {
			// Only documented surface: exported items. (`opaque` exports the
			// type but hides constructors — still worth a doc entry.)
			if !matches!(def.visibility, Visibility::Public | Visibility::Opaque) {
				continue;
			}
			let doc = doc_comment_for(module, def.range.start.line);
			let item = match &def.kind {
				// A value def's real type lives on its body expr; `def.ty` is
				// left an unconstrained var by the analyzer (matches hover).
				DefinitionKind::Expr(expr) => ItemDoc {
					name: def.name.name.clone(),
					// The type printer renders type variables ML-style (`'a`);
					// Pluma source writes them bare (`a`), so match the language.
					signature: format!("{}", expr.ty).replace('\'', ""),
					blocks: doc.map(|d| split_blocks(&d)).unwrap_or_default(),
				},
				// Instances have no name worth documenting on their own.
				DefinitionKind::Instance(_) => continue,
				// Enums/traits/aliases have no printable `Type`; show the
				// declaration verbatim from source — header as the signature,
				// full body as a leading code block (variants, methods, fields).
				_ => {
					let decl = slice(source, &def.range);
					let signature = header_of(&decl);
					let mut blocks = vec![code_block(decl)];
					if let Some(d) = doc {
						blocks.extend(split_blocks(&d));
					}
					ItemDoc {
						name: def.name.name.clone(),
						signature,
						blocks,
					}
				}
			};
			items.push(item);
		}
	}

	let blocks = module_doc_comment(module)
		.map(|d| split_blocks(&d))
		.unwrap_or_default();

	ModuleDoc {
		name: module_name.to_string(),
		namespace,
		blocks,
		items,
	}
}

/// Split a doc comment into prose paragraphs and example code blocks. After
/// `doc_comment_for` strips the conventional leading space, an example line
/// (written `#     list.length …` in source) keeps a 4-space or tab indent;
/// that's the signal for a code block. Blank lines separate paragraphs.
fn split_blocks(doc: &str) -> Vec<DocBlock> {
	let mut blocks: Vec<DocBlock> = Vec::new();
	let mut buf: Vec<String> = Vec::new();
	let mut in_code = false;

	let flush = |buf: &mut Vec<String>, in_code: bool, blocks: &mut Vec<DocBlock>| {
		if buf.is_empty() {
			return;
		}
		if in_code {
			blocks.push(code_block(buf.join("\n")));
		} else {
			// Prose lines are hard-wrapped in source; rejoin with spaces so the
			// renderer reflows them.
			blocks.push(prose_block(buf.join(" ")));
		}
		buf.clear();
	};

	for line in doc.lines() {
		if line.trim().is_empty() {
			flush(&mut buf, in_code, &mut blocks);
			continue;
		}
		let is_code = line.starts_with("    ") || line.starts_with('\t');
		if !buf.is_empty() && is_code != in_code {
			flush(&mut buf, in_code, &mut blocks);
		}
		in_code = is_code;
		let text = if is_code {
			line
				.strip_prefix("    ")
				.or_else(|| line.strip_prefix('\t'))
				.unwrap_or(line)
				.to_string()
		} else {
			line.trim().to_string()
		};
		buf.push(text);
	}
	flush(&mut buf, in_code, &mut blocks);
	blocks
}

// The text of a top-level declaration, verbatim from source, with the leading
// `public`/`opaque` keyword trimmed. Used for the type/trait/alias defs the
// type printer can't render.
fn slice(source: &str, range: &Range) -> String {
	let starts = line_starts(source);
	let start = starts
		.get(range.start.line)
		.map(|s| s + range.start.col)
		.unwrap_or(0);
	let end = starts
		.get(range.end.line)
		.map(|s| s + range.end.col)
		.unwrap_or(source.len())
		.min(source.len());
	let raw = source.get(start..end).unwrap_or("").trim();
	strip_visibility(raw).to_string()
}

// The one-line header of a declaration: everything up to the opening brace (or
// the whole first line for a brace-less alias).
fn header_of(decl: &str) -> String {
	let before_brace = decl.split('{').next().unwrap_or(decl);
	before_brace
		.lines()
		.next()
		.unwrap_or(before_brace)
		.trim()
		.to_string()
}

fn strip_visibility(s: &str) -> &str {
	let s = s.trim_start();
	s.strip_prefix("public ")
		.or_else(|| s.strip_prefix("opaque "))
		.unwrap_or(s)
		.trim_start()
}

// Byte offset of the start of each line.
fn line_starts(source: &str) -> Vec<usize> {
	let mut starts = vec![0];
	for (i, b) in source.bytes().enumerate() {
		if b == b'\n' {
			starts.push(i + 1);
		}
	}
	starts
}

/// Serialize one or more module docs as a JSON array — the doc *data artifact*.
/// A docs renderer (the website's `/std` pages, and eventually any published
/// package's docs) reads this at runtime and renders it, so the docs come
/// straight from source and stay honest. The shape mirrors the renderer's record
/// types: `[{name, namespace, blocks, items}, …]`, where each block is
/// `{kind, text, spans}` and each item `{name, signature, blocks}`.
pub fn to_json(modules: &[ModuleDoc]) -> String {
	let mut out = String::new();
	out.push('[');
	for (i, m) in modules.iter().enumerate() {
		if i > 0 {
			out.push(',');
		}
		out.push_str("{\"name\":");
		push_json_str(&mut out, &m.name);
		out.push_str(",\"namespace\":");
		push_json_str(&mut out, &m.namespace);
		out.push_str(",\"blocks\":");
		push_blocks_json(&mut out, &m.blocks);
		out.push_str(",\"items\":[");
		for (j, it) in m.items.iter().enumerate() {
			if j > 0 {
				out.push(',');
			}
			out.push_str("{\"name\":");
			push_json_str(&mut out, &it.name);
			out.push_str(",\"signature\":");
			push_json_str(&mut out, &it.signature);
			out.push_str(",\"blocks\":");
			push_blocks_json(&mut out, &it.blocks);
			out.push('}');
		}
		out.push_str("]}");
	}
	out.push(']');
	out
}

fn push_blocks_json(out: &mut String, blocks: &[DocBlock]) {
	out.push('[');
	for (i, b) in blocks.iter().enumerate() {
		if i > 0 {
			out.push(',');
		}
		out.push_str("{\"kind\":");
		push_json_str(out, b.kind);
		out.push_str(",\"text\":");
		push_json_str(out, &b.text);
		out.push_str(",\"spans\":[");
		for (j, s) in b.spans.iter().enumerate() {
			if j > 0 {
				out.push(',');
			}
			out.push_str("{\"class\":");
			push_json_str(out, &s.class);
			out.push_str(",\"text\":");
			push_json_str(out, &s.text);
			out.push('}');
		}
		out.push_str("]}");
	}
	out.push(']');
}

/// Append `s` as a JSON string literal — RFC 8259 escaping: `"` and `\` backslashed,
/// the named short escapes for the common control chars, and `\u00XX` for the rest.
fn push_json_str(out: &mut String, s: &str) {
	out.push('"');
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\t' => out.push_str("\\t"),
			'\r' => out.push_str("\\r"),
			c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
			c => out.push(c),
		}
	}
	out.push('"');
}

// --- doc-comment association (shared with the LSP's hover) -------------------

/// The module-level doc comment: a comment block at the very top of the file,
/// separated from the first definition by a blank line so it isn't the first
/// def's own doc. Returns `None` if the file opens with code, or if the leading
/// comment block butts directly against the first item (then it belongs to that
/// item, and `doc_comment_for` already shows it there).
pub fn module_doc_comment(module: &Module) -> Option<String> {
	let ast = module.ast.as_ref()?;
	module.comments.get(&0)?;

	let mut lines: Vec<String> = Vec::new();
	let mut line = 0usize;
	while let Some(text) = module.comments.get(&line) {
		lines.push(strip_comment(text));
		line += 1;
	}

	// The block is a module doc only when a blank line separates it from the
	// first item — i.e. the earliest top-level item starts strictly after
	// `line`. If an item sits on `line` (directly attached), the block belongs
	// to that item, not the module.
	if let Some(item_line) = first_item_line(ast) {
		if item_line <= line {
			return None;
		}
	}
	Some(lines.join("\n"))
}

/// The doc comment for a top-level def: the contiguous run of full-line
/// comments directly above it, bounded at the previous top-level item's end
/// line so a trailing comment on the line above is never mistaken for this
/// def's doc.
pub fn doc_comment_for(module: &Module, def_start_line: usize) -> Option<String> {
	let ast = module.ast.as_ref()?;

	let mut floor: isize = -1;
	for u in &ast.uses {
		if u.range.end.line < def_start_line {
			floor = floor.max(u.range.end.line as isize);
		}
	}
	for d in &ast.body {
		if d.range.end.line < def_start_line {
			floor = floor.max(d.range.end.line as isize);
		}
	}

	let mut lines: Vec<String> = Vec::new();
	let mut line = def_start_line as isize - 1;
	while line > floor {
		let Some(text) = module.comments.get(&(line as usize)) else {
			break;
		};
		lines.push(strip_comment(text));
		line -= 1;
	}

	if lines.is_empty() {
		return None;
	}
	lines.reverse();
	Some(lines.join("\n"))
}

// Comment text is everything after `#`; drop the conventional single leading
// space so `# foo` renders as `foo` (and `#     code` keeps its indent).
fn strip_comment(text: &str) -> String {
	text
		.strip_prefix(' ')
		.unwrap_or(text)
		.trim_end()
		.to_string()
}

/// The start line of the earliest top-level item (`use` or `def`) in the file.
fn first_item_line(ast: &ModuleNode) -> Option<usize> {
	let uses = ast.uses.iter().map(|u| u.range.start.line);
	let defs = ast.body.iter().map(|d| d.range.start.line);
	uses.chain(defs).min()
}
