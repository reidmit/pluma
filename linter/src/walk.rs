//! The lint traversal. A single pre-order walk over every expression in a
//! module, offering each `ExprNode` to every rule. Centralizing it here means a
//! rule never re-implements traversal — it just inspects the node it's handed.

use crate::Rule;
use compiler::Diagnostic;
use compiler::ast::{DefinitionKind, DefinitionNode, ExprKind, ExprNode, ModuleNode, TryNode};

pub fn walk_module(ast: &ModuleNode, rules: &[Box<dyn Rule>], out: &mut Vec<Diagnostic>) {
	for def in &ast.body {
		walk_definition(def, rules, out);
	}
}

fn walk_definition(def: &DefinitionNode, rules: &[Box<dyn Rule>], out: &mut Vec<Diagnostic>) {
	match &def.kind {
		DefinitionKind::Expr(expr) => visit_expr(expr, rules, out),
		// Trait method defaults and instance method bodies are ordinary
		// expression bodies, so lints apply inside them too.
		DefinitionKind::Trait(t) => {
			for method in &t.methods {
				if let Some(default) = &method.default {
					visit_expr(default, rules, out);
				}
			}
		}
		DefinitionKind::Instance(inst) => {
			for method in &inst.methods {
				walk_definition(method, rules, out);
			}
		}
		// Type aliases and enum declarations carry no expressions.
		DefinitionKind::Alias(_) | DefinitionKind::Enum(_) => {}
	}
}

/// Visit a statement block: offer it to the body-level rules, then recurse into
/// each statement. Use this only for real blocks (function/control-flow bodies),
/// where the elements are sequential statements — not for argument or element
/// lists, which use [`visit_each`].
fn visit_body(body: &[ExprNode], rules: &[Box<dyn Rule>], out: &mut Vec<Diagnostic>) {
	for rule in rules {
		rule.check_body(body, out);
	}
	visit_each(body, rules, out);
}

/// Recurse into each expression in a list without treating the list as a block.
/// For call arguments, tuple/list elements, interpolation parts — sequences that
/// share a `Vec<ExprNode>` shape with a body but carry no statement semantics.
fn visit_each(exprs: &[ExprNode], rules: &[Box<dyn Rule>], out: &mut Vec<Diagnostic>) {
	for expr in exprs {
		visit_expr(expr, rules, out);
	}
}

/// Offer `expr` to every rule, then recurse into its sub-expressions. Pre-order,
/// so a rule sees an outer node before its children.
fn visit_expr(expr: &ExprNode, rules: &[Box<dyn Rule>], out: &mut Vec<Diagnostic>) {
	for rule in rules {
		rule.check_expr(expr, out);
	}

	match &expr.kind {
		ExprKind::BinaryOperation { left, right, .. } => {
			visit_expr(left, rules, out);
			visit_expr(right, rules, out);
		}
		ExprKind::UnaryOperation { right, .. } => visit_expr(right, rules, out),
		ExprKind::ElementAccess { receiver, .. } => visit_expr(receiver, rules, out),
		ExprKind::FieldAccess { receiver, .. } => visit_expr(receiver, rules, out),
		ExprKind::Fun(fun) => visit_body(&fun.body, rules, out),
		ExprKind::Call(call) => {
			visit_expr(&call.callee, rules, out);
			visit_each(&call.args, rules, out);
		}
		ExprKind::Grouping(inner) => visit_expr(inner, rules, out),
		ExprKind::Interpolation(parts) => visit_each(parts, rules, out),
		ExprKind::Let(let_node) => visit_expr(&let_node.value, rules, out),
		ExprKind::Defer(inner) => visit_expr(inner, rules, out),
		ExprKind::Record(fields) => {
			for (_, value) in fields {
				visit_expr(value, rules, out);
			}
		}
		ExprKind::RecordUpdate { base, fields } => {
			visit_expr(base, rules, out);
			for (_, value) in fields {
				visit_expr(value, rules, out);
			}
		}
		ExprKind::Tuple(entries) => visit_each(entries, rules, out),
		ExprKind::Try(TryNode { value, rest, .. }) => {
			visit_expr(value, rules, out);
			visit_body(rest, rules, out);
		}
		ExprKind::List(items) => {
			for item in items {
				visit_expr(item.expr(), rules, out);
			}
		}
		ExprKind::If(if_node) => {
			visit_expr(&if_node.subject, rules, out);
			visit_body(&if_node.body, rules, out);
			if let Some(else_body) = &if_node.else_body {
				visit_body(else_body, rules, out);
			}
		}
		ExprKind::When(when_node) => {
			visit_expr(&when_node.subject, rules, out);
			for case in &when_node.cases {
				visit_body(&case.body, rules, out);
			}
		}
		ExprKind::While(while_node) => {
			visit_expr(&while_node.subject, rules, out);
			visit_body(&while_node.body, rules, out);
		}
		ExprKind::Scope(scope_node) => visit_body(&scope_node.body, rules, out),
		ExprKind::Using { body, .. } => visit_body(body, rules, out),
		// Leaves: no sub-expressions to descend into.
		ExprKind::NamespaceAccess(_)
		| ExprKind::EmptyTuple
		| ExprKind::Identifier(_)
		| ExprKind::Literal(_)
		| ExprKind::Regex(_)
		| ExprKind::Builtin(_)
		| ExprKind::ImplicitMember { .. } => {}
	}
}
