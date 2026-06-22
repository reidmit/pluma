use crate::{Context, Finding, Rule};
use compiler::ast::{CaseNode, ExprKind, ExprNode, LiteralKind, PatternKind, WhenNode};
use compiler::{Diagnostic, Point, Range, Reportable};

/// Flags a `when` that isn't doing exhaustive dispatch — it has exactly two arms
/// and either a wildcard catch-all (`when x is some v { … } is _ { … }`) or a
/// boolean test (`when b is true { … } is false { … }`). Both are an `if`/`else`
/// in disguise: the `_` is what `else` already means, and a two-way boolean split
/// is the plainest `if`.
///
/// A two-arm `when` whose arms both name real variants (`is some v` / `is none`)
/// is left alone — there the exhaustiveness check is doing real work, which is
/// what `when` is for. The linter runs on the parsed AST before name resolution,
/// where a bare nullary variant (`none`) and a plain binding (`other`) are the
/// same `Identifier` pattern, so only the unambiguous `_` counts as a catch-all;
/// a binding-named catch-all is left alone rather than guessed at.
pub struct WhenAsIf;

impl Rule for WhenAsIf {
	fn name(&self) -> &'static str {
		"when-as-if"
	}

	fn check_expr(&self, expr: &ExprNode, _ctx: &Context, out: &mut Vec<Finding>) {
		let ExprKind::When(node) = &expr.kind else {
			return;
		};
		// Exactly two arms: a third arm makes this multi-way dispatch, which
		// collapses to an `if`/`else if` chain, not a single `if` — that's the
		// opposite direction (keep the `when`).
		let [first, last] = node.cases.as_slice() else {
			return;
		};

		if is_bool_literal(&first.pattern.kind) && is_bool_literal(&last.pattern.kind) {
			// A two-way boolean split. Report-only: collapsing `is true { A } is
			// false { B }` to `if subj { A } else { B }` means deleting the `is true`
			// and rewriting `is false` — and the arms can be in either order, so
			// the human gets it right with less risk than a span rewrite would.
			out.push(Finding::new(
				Diagnostic::report_warning(Lint::Boolean).with_span(expr.range),
			));
		} else if is_catch_all(&last.pattern.kind) {
			// `when subj is <pat> { A } is _ { B }` is exactly `if subj is <pat> { A }
			// else { B }`: the first arm carries over verbatim and the wildcard
			// becomes the `else`. The rewrite is two narrow edits — swap the `when`
			// keyword for `if`, and (when the catch-all was written `is _` rather
			// than `else`) replace that arm's `is _` with `else`.
			out.push(catch_all_finding(node, last));
		}
		// Two arms that both name real variants: a genuine exhaustive match. Left
		// alone — there the exhaustiveness check is doing real work.
	}
}

/// The catch-all finding plus its autofix edits.
fn catch_all_finding(node: &WhenNode, last: &CaseNode) -> Finding {
	let finding = Finding::new(Diagnostic::report_warning(Lint::CatchAll).with_span(node.range))
		.with_fix(
			// The `when` keyword is a fixed four characters at the node's start.
			Range::between(
				node.range.start,
				Point::at(node.range.start.line, node.range.start.col + "when".len()),
			),
			"if",
		);

	// `else` desugars to an `is _` whose pattern range *is* the `else` keyword, so
	// the case and its pattern share a start. When they differ, the source wrote
	// `is _` literally, which becomes `else`.
	let wrote_else = last.range.start.line == last.pattern.range.start.line
		&& last.range.start.col == last.pattern.range.start.col;
	if wrote_else {
		finding
	} else {
		finding.with_fix(
			Range::between(last.range.start, last.pattern.range.end),
			"else",
		)
	}
}

/// The wildcard `_` pattern — the unambiguous catch-all. This is what an `else`
/// arm already expresses, so a `when` ending in one isn't enumerating variants —
/// it's testing one case and catching the rest. A binding catch-all (`is other`)
/// is deliberately excluded: pre-resolution it's indistinguishable from a bare
/// nullary variant (`is none`), which is a real arm, not a catch-all.
fn is_catch_all(kind: &PatternKind) -> bool {
	matches!(kind, PatternKind::Underscore)
}

/// A `true`/`false` literal pattern.
fn is_bool_literal(kind: &PatternKind) -> bool {
	matches!(kind, PatternKind::Literal(lit) if matches!(lit.kind, LiteralKind::Bool(_)))
}

enum Lint {
	CatchAll,
	Boolean,
}

impl std::fmt::Display for Lint {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Lint::CatchAll => write!(
				f,
				"This `when` has only two arms with a catch-all — that's an `if`/`else`."
			),
			Lint::Boolean => write!(
				f,
				"This `when` only tests a boolean — that's an `if`/`else`."
			),
		}
	}
}

impl Reportable for Lint {
	fn code(&self) -> &'static str {
		"L0011"
	}

	fn help(&self) -> Option<String> {
		Some(
			match self {
				Lint::CatchAll => {
					"rewrite as `if <subject> is <pattern> { … } else { … }`; reserve `when` for \
					 exhaustive matches over a value's variants."
				}
				Lint::Boolean => {
					"rewrite as `if <subject> { … } else { … }`; reserve `when` for exhaustive \
					 matches over a value's variants."
				}
			}
			.to_string(),
		)
	}
}
