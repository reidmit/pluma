//! Structural equality over expressions, ignoring spans and inferred types.
//! Used by the identical-branches lint to tell whether two `if` arms are the
//! same code. Conservative by design: shapes it doesn't model (control flow,
//! function literals, regexes) compare as *not equal*, so the lint under-reports
//! rather than firing on a false match.

use compiler::ast::{ExprKind, ExprNode, ListItem, LiteralKind};

/// Two statement blocks are equal when they have the same length and each
/// statement is structurally equal.
pub fn bodies_eq(a: &[ExprNode], b: &[ExprNode]) -> bool {
	a.len() == b.len() && a.iter().zip(b).all(|(x, y)| expr_eq(x, y))
}

/// Structural equality of two expressions, ignoring spans and types. Grouping
/// parens are transparent, so `(foo)` equals `foo`.
pub fn expr_eq(a: &ExprNode, b: &ExprNode) -> bool {
	let a = strip_grouping(a);
	let b = strip_grouping(b);

	match (&a.kind, &b.kind) {
		(ExprKind::Literal(x), ExprKind::Literal(y)) => literal_eq(&x.kind, &y.kind),
		(ExprKind::Identifier(x), ExprKind::Identifier(y)) => x.name == y.name,
		(ExprKind::EmptyTuple, ExprKind::EmptyTuple) => true,
		(ExprKind::Builtin(x), ExprKind::Builtin(y)) => x == y,

		(
			ExprKind::BinaryOperation {
				op: ox,
				left: lx,
				right: rx,
			},
			ExprKind::BinaryOperation {
				op: oy,
				left: ly,
				right: ry,
			},
		) => {
			std::mem::discriminant(&ox.kind) == std::mem::discriminant(&oy.kind)
				&& expr_eq(lx, ly)
				&& expr_eq(rx, ry)
		}

		(
			ExprKind::UnaryOperation { op: ox, right: rx },
			ExprKind::UnaryOperation { op: oy, right: ry },
		) => std::mem::discriminant(ox) == std::mem::discriminant(oy) && expr_eq(rx, ry),

		(
			ExprKind::ElementAccess {
				receiver: rx,
				index: ix,
			},
			ExprKind::ElementAccess {
				receiver: ry,
				index: iy,
			},
		) => ix == iy && expr_eq(rx, ry),

		(
			ExprKind::FieldAccess {
				receiver: rx,
				field: fx,
			},
			ExprKind::FieldAccess {
				receiver: ry,
				field: fy,
			},
		) => fx.name == fy.name && expr_eq(rx, ry),

		(ExprKind::NamespaceAccess(px), ExprKind::NamespaceAccess(py)) => {
			px.len() == py.len() && px.iter().zip(py).all(|(x, y)| x.name == y.name)
		}

		(ExprKind::Call(cx), ExprKind::Call(cy)) => {
			expr_eq(&cx.callee, &cy.callee) && exprs_eq(&cx.args, &cy.args)
		}

		(ExprKind::Tuple(xs), ExprKind::Tuple(ys)) => exprs_eq(xs, ys),

		(ExprKind::Interpolation(xs), ExprKind::Interpolation(ys)) => exprs_eq(xs, ys),

		(ExprKind::Record(fx), ExprKind::Record(fy)) => fields_eq(fx, fy),

		(
			ExprKind::RecordUpdate {
				base: bx,
				fields: fx,
			},
			ExprKind::RecordUpdate {
				base: by,
				fields: fy,
			},
		) => expr_eq(bx, by) && fields_eq(fx, fy),

		(ExprKind::List(xs), ExprKind::List(ys)) => {
			xs.len() == ys.len()
				&& xs.iter().zip(ys).all(|(x, y)| {
					matches!(x, ListItem::Spread(_)) == matches!(y, ListItem::Spread(_))
						&& expr_eq(x.expr(), y.expr())
				})
		}

		// Everything else (control flow, `fun`, `let`, `try`, `scope`, `using`,
		// regexes) is treated as unequal: matching them soundly needs more shape
		// modeling than the lint warrants, and a false negative is harmless.
		_ => false,
	}
}

fn exprs_eq(a: &[ExprNode], b: &[ExprNode]) -> bool {
	a.len() == b.len() && a.iter().zip(b).all(|(x, y)| expr_eq(x, y))
}

fn fields_eq(
	a: &[(compiler::ast::IdentifierNode, ExprNode)],
	b: &[(compiler::ast::IdentifierNode, ExprNode)],
) -> bool {
	a.len() == b.len()
		&& a
			.iter()
			.zip(b)
			.all(|((nx, vx), (ny, vy))| nx.name == ny.name && expr_eq(vx, vy))
}

fn strip_grouping(expr: &ExprNode) -> &ExprNode {
	match &expr.kind {
		ExprKind::Grouping(inner) => strip_grouping(inner),
		_ => expr,
	}
}

fn literal_eq(a: &LiteralKind, b: &LiteralKind) -> bool {
	use LiteralKind::*;
	match (a, b) {
		(Bool(x), Bool(y)) => x == y,
		(FloatDecimal(x), FloatDecimal(y)) => x == y,
		(Duration(x), Duration(y)) => x == y,
		(String(x, _), String(y, _)) => x == y,
		(Bytes(x), Bytes(y)) => x == y,
		// Integers compare by value regardless of the base they were written in.
		(
			IntDecimal(x) | IntOctal(x) | IntHex(x) | IntBinary(x),
			IntDecimal(y) | IntOctal(y) | IntHex(y) | IntBinary(y),
		) => x == y,
		_ => false,
	}
}
