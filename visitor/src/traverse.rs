use crate::visitor::Visitor;
use ast::*;

pub trait Traverse {
	fn traverse<V: Visitor>(&self, _visitor: &mut V) {}
}

impl Traverse for BlockNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_block(self);

		if let Some(param) = &self.param {
			param.traverse(visitor);
		}

		for stmt in &self.body {
			stmt.traverse(visitor);
		}

		visitor.leave_block(self);
	}
}

impl Traverse for CallNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_call(self);

		// for arg in &self.args {
		//   arg.traverse(visitor);
		// }

		// self.callee.traverse(visitor);

		visitor.leave_call(self);
	}
}

impl Traverse for DefNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_def(self);

		// match &self.kind {
		//   DefKind::Function { signature } => {
		//     for (ident, type_expr) in signature {
		//       ident.traverse(visitor);
		//       type_expr.traverse(visitor);
		//     }
		//   }

		//   DefKind::Method {
		//     receiver,
		//     signature,
		//   } => {
		//     for (ident, type_expr) in signature {
		//       ident.traverse(visitor);
		//       type_expr.traverse(visitor);
		//     }

		//     receiver.traverse(visitor);
		//   }
		// }

		self.block.traverse(visitor);

		visitor.leave_def(self);
	}
}

impl Traverse for ExprNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_expr(self);

		match &self.kind {
			ExprKind::Access { receiver, property } => {
				receiver.traverse(visitor);
				property.traverse(visitor);
			}

			ExprKind::Assignment { left, right } => {
				right.traverse(visitor);
				left.traverse(visitor);
			}

			ExprKind::BinaryOperation { op, left, right } => {
				op.traverse(visitor);
				left.traverse(visitor);
				right.traverse(visitor);
			}

			ExprKind::Block { block } => block.traverse(visitor),

			ExprKind::Call { call } => call.traverse(visitor),

			ExprKind::Literal { literal } => literal.traverse(visitor),

			ExprKind::Grouping { inner } => inner.traverse(visitor),

			ExprKind::Identifier { ident } => ident.traverse(visitor),

			ExprKind::Match { match_ } => match_.traverse(visitor),

			ExprKind::UnaryOperation { op, right } => {
				op.traverse(visitor);
				right.traverse(visitor);
			}

			ExprKind::Interpolation { parts } => {
				for part in parts {
					part.traverse(visitor);
				}
			}

			ExprKind::Tuple { entries } => {
				for (label, value) in entries {
					if let Some(ident) = label {
						ident.traverse(visitor);
					}

					value.traverse(visitor);
				}
			}

			ExprKind::EmptyTuple => {}

			ExprKind::MultiPartIdentifier { parts } => {
				for part in parts {
					part.traverse(visitor);
				}
			}

			ExprKind::TypeAssertion {
				expr,
				asserted_type,
			} => {
				expr.traverse(visitor);
				asserted_type.traverse(visitor);
			}

			_other_kind => todo!("traverse other kind"),
		}

		visitor.leave_expr(self);
	}
}

impl Traverse for IdentifierNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_identifier(self);

		visitor.leave_identifier(self);
	}
}

impl Traverse for LetNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_let(self);

		self.pattern.traverse(visitor);
		self.value.traverse(visitor);

		visitor.leave_let(self);
	}
}

impl Traverse for LiteralNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_literal(self);

		visitor.leave_literal(self);
	}
}

impl Traverse for MatchNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_match(self);

		self.subject.traverse(visitor);

		for case in &self.cases {
			case.traverse(visitor);
		}

		visitor.leave_match(self);
	}
}

impl Traverse for MatchCaseNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_match_case(self);

		self.pattern.traverse(visitor);
		self.body.traverse(visitor);

		visitor.leave_match_case(self);
	}
}

impl Traverse for ModuleNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_module(self);

		for node in &self.body {
			node.traverse(visitor);
		}

		visitor.leave_module(self);
	}
}

impl Traverse for OperatorNode {
	// todo
}

impl Traverse for PatternNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_pattern(self);

		// ?

		visitor.leave_pattern(self);
	}
}

impl Traverse for StatementNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_statement(self);

		match &self.kind {
			StatementKind::Let(node) => node.traverse(visitor),
			StatementKind::Def(node) => node.traverse(visitor),
			StatementKind::Type(node) => node.traverse(visitor),
			StatementKind::Expr(node) => node.traverse(visitor),
		};

		visitor.leave_statement(self);
	}
}

impl Traverse for TypeExprNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_type_expr(self);

		visitor.leave_type_expr(self);
	}
}

impl Traverse for TypeDefNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_type_def(self);

		visitor.leave_type_def(self);
	}
}

impl Traverse for TypeIdentifierNode {
	fn traverse<V: Visitor>(&self, visitor: &mut V) {
		visitor.enter_type_identifier(self);

		visitor.leave_type_identifier(self);
	}
}
