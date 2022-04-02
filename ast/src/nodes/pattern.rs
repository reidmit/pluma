use super::*;
use crate::common::*;
use crate::value_type::ValueType;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct PatternNode {
	pub pos: Position,
	pub kind: PatternKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum PatternKind {
	// e.g. let x = / let mut x
	Identifier(IdentifierNode, bool),
	// e.g. let Person (x, y) =
	Constructor(IdentifierNode, Box<PatternNode>),
	// e.g. let (x, y: b) =
	Tuple(Vec<(Option<IdentifierNode>, PatternNode)>),
	// e.g. '_' in let (x, _) =
	Underscore,
	// e.g. '1' in match x | 1 => "yes" | _ => "no"
	Literal(LiteralNode),
	// e.g. match str | "$(thing)?" => "yes" | _ => "no"
	Interpolation(Vec<ExprNode>),
}

impl PatternNode {
	pub fn to_expr(self) -> ExprNode {
		let pos = self.pos;

		let expr_kind = match self.kind {
			PatternKind::Identifier(ident, _) => ExprKind::Identifier { ident },

			PatternKind::Literal(literal) => ExprKind::Literal { literal },

			PatternKind::Interpolation(parts) => ExprKind::Interpolation { parts },

			PatternKind::Tuple(entry_patterns) => {
				let mut entries = Vec::new();

				for (label, pat) in entry_patterns {
					entries.push((label, pat.to_expr()))
				}

				ExprKind::Tuple { entries }
			}

			other => todo!("other expr kind in pattern: {:#?}", other),
		};

		ExprNode {
			pos,
			kind: expr_kind,
			typ: ValueType::Unknown,
		}
	}

	pub fn to_statement(self) -> Option<StatementNode> {
		let pos = self.pos;
		let expr = self.to_expr();

		Some(StatementNode {
			pos,
			kind: StatementKind::Expr(expr),
		})
	}
}
