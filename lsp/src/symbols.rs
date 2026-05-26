use compiler::ast::*;
use compiler::{Diagnostic, Module};
use std::path::PathBuf;
use tower_lsp::lsp_types::{DocumentSymbol, Position, Range as LspRange, SymbolKind};

// A document outline built from parser output: one top-level symbol per
// `def`, with enum variants and trait methods nested as children. Powers
// the editor's outline view and breadcrumb bar. Parse-only, like
// `semantic_tokens` and `goto` — no analyzer needed.

pub fn document_symbols(source: &[u8]) -> Vec<DocumentSymbol> {
	let mut module = Module::new("<lsp>".to_string(), PathBuf::new());
	let mut diagnostics: Vec<Diagnostic> = Vec::new();
	module.parse_from_bytes(source.to_vec(), &mut diagnostics);

	let Some(ast) = module.ast.as_ref() else {
		return vec![];
	};

	let mut out = Vec::new();
	for def in &ast.body {
		if let Some(sym) = def_symbol(def) {
			out.push(sym);
		}
	}
	out
}

fn def_symbol(d: &DefinitionNode) -> Option<DocumentSymbol> {
	match &d.kind {
		DefinitionKind::Expr(expr) => {
			let kind = if matches!(expr.kind, ExprKind::Fun(_)) {
				SymbolKind::FUNCTION
			} else {
				SymbolKind::VARIABLE
			};
			Some(symbol(&d.name.name, kind, d.range, d.name.range, None))
		}
		DefinitionKind::Alias(_) => Some(symbol(
			&d.name.name,
			SymbolKind::STRUCT,
			d.range,
			d.name.range,
			None,
		)),
		DefinitionKind::Enum(en) => {
			let children = en
				.variants
				.iter()
				.map(|v| {
					symbol(
						&v.name.name,
						SymbolKind::ENUM_MEMBER,
						v.range,
						v.name.range,
						None,
					)
				})
				.collect();
			Some(symbol(
				&d.name.name,
				SymbolKind::ENUM,
				d.range,
				d.name.range,
				Some(children),
			))
		}
		DefinitionKind::Trait(t) => {
			let children = t
				.methods
				.iter()
				.map(|m| {
					symbol(
						&m.name.name,
						SymbolKind::METHOD,
						m.range,
						m.name.range,
						None,
					)
				})
				.collect();
			Some(symbol(
				&d.name.name,
				SymbolKind::INTERFACE,
				d.range,
				d.name.range,
				Some(children),
			))
		}
		DefinitionKind::Instance(inst) => {
			let children = inst.methods.iter().filter_map(def_symbol).collect();
			Some(symbol(
				&inst.trait_name.name,
				SymbolKind::INTERFACE,
				d.range,
				inst.trait_name.range,
				Some(children),
			))
		}
	}
}

#[allow(deprecated)] // `deprecated` field is required by the struct literal.
fn symbol(
	name: &str,
	kind: SymbolKind,
	range: compiler::Range,
	selection: compiler::Range,
	children: Option<Vec<DocumentSymbol>>,
) -> DocumentSymbol {
	DocumentSymbol {
		name: name.to_string(),
		detail: None,
		kind,
		tags: None,
		deprecated: None,
		range: to_lsp(range),
		// The selection range must be contained in `range`; the name span
		// always is.
		selection_range: to_lsp(selection),
		children,
	}
}

fn to_lsp(r: compiler::Range) -> LspRange {
	LspRange {
		start: Position {
			line: r.start.line as u32,
			character: r.start.col as u32,
		},
		end: Position {
			line: r.end.line as u32,
			character: r.end.col as u32,
		},
	}
}
