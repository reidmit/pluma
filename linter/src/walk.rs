//! The lint traversal. A single pre-order walk over every expression in a
//! module, offering each `ExprNode` to every rule. Centralizing it here means a
//! rule never re-implements traversal — it just inspects the node it's handed.
//!
//! The walk also maintains a [`Context`] of the local-value bindings in scope
//! (function parameters, `scope as` handles, `let` bindings), so a rule can tell
//! a projection on a runtime value from one on a namespace.

use crate::{Context, Finding, Rule};
use compiler::ast::{
	DefinitionKind, DefinitionNode, ExprKind, ExprNode, ModuleNode, PatternKind, PatternNode, TryNode,
};

pub fn walk_module(
	ast: &ModuleNode,
	rules: &[Box<dyn Rule>],
	ctx: &mut Context,
	out: &mut Vec<Finding>,
) {
	for def in &ast.body {
		walk_definition(def, rules, ctx, out);
	}
}

fn walk_definition(
	def: &DefinitionNode,
	rules: &[Box<dyn Rule>],
	ctx: &mut Context,
	out: &mut Vec<Finding>,
) {
	match &def.kind {
		DefinitionKind::Expr(expr) => visit_expr(expr, rules, ctx, out),
		// Trait method defaults and instance method bodies are ordinary
		// expression bodies, so lints apply inside them too.
		DefinitionKind::Trait(t) => {
			for method in &t.methods {
				if let Some(default) = &method.default {
					visit_expr(default, rules, ctx, out);
				}
			}
		}
		DefinitionKind::Instance(inst) => {
			for method in &inst.methods {
				walk_definition(method, rules, ctx, out);
			}
		}
		// Type aliases and enum declarations carry no expressions.
		DefinitionKind::Alias(_) | DefinitionKind::Enum(_) => {}
	}
}

/// Visit a statement block: offer it to the body-level rules, then recurse into
/// each statement. Pushes a frame so the block's `let` bindings come into scope
/// as the walk proceeds. Use only for real blocks (function/control-flow
/// bodies), not argument or element lists (use [`visit_each`]).
fn visit_body(
	body: &[ExprNode],
	rules: &[Box<dyn Rule>],
	ctx: &mut Context,
	out: &mut Vec<Finding>,
) {
	for rule in rules {
		rule.check_body(body, ctx, out);
	}
	ctx.push(Vec::new());
	for expr in body {
		visit_expr(expr, rules, ctx, out);
		// A `let` binds its name for the statements that follow it.
		if let ExprKind::Let(let_node) = &expr.kind {
			bind_pattern(&let_node.pattern, ctx);
		}
	}
	ctx.pop();
}

/// Recurse into each expression in a list without treating the list as a block.
/// For call arguments, tuple/list elements, interpolation parts — sequences that
/// share a `Vec<ExprNode>` shape with a body but carry no statement semantics.
fn visit_each(
	exprs: &[ExprNode],
	rules: &[Box<dyn Rule>],
	ctx: &mut Context,
	out: &mut Vec<Finding>,
) {
	for expr in exprs {
		visit_expr(expr, rules, ctx, out);
	}
}

/// Offer `expr` to every rule, then recurse into its sub-expressions. Pre-order,
/// so a rule sees an outer node before its children.
fn visit_expr(expr: &ExprNode, rules: &[Box<dyn Rule>], ctx: &mut Context, out: &mut Vec<Finding>) {
	for rule in rules {
		rule.check_expr(expr, ctx, out);
	}

	match &expr.kind {
		ExprKind::BinaryOperation { left, right, .. } => {
			visit_expr(left, rules, ctx, out);
			visit_expr(right, rules, ctx, out);
		}
		ExprKind::UnaryOperation { right, .. } => visit_expr(right, rules, ctx, out),
		ExprKind::ElementAccess { receiver, .. } => visit_expr(receiver, rules, ctx, out),
		ExprKind::FieldAccess { receiver, .. } => visit_expr(receiver, rules, ctx, out),
		ExprKind::Fun(fun) => {
			// Parameters are local values within the body.
			let params = fun.params.iter().map(|p| p.ident.name.clone()).collect();
			ctx.push(params);
			visit_body(&fun.body, rules, ctx, out);
			ctx.pop();
		}
		ExprKind::Call(call) => {
			visit_expr(&call.callee, rules, ctx, out);
			visit_each(&call.args, rules, ctx, out);
		}
		ExprKind::Grouping(inner) => visit_expr(inner, rules, ctx, out),
		ExprKind::Interpolation(parts) => visit_each(parts, rules, ctx, out),
		ExprKind::Let(let_node) => visit_expr(&let_node.value, rules, ctx, out),
		ExprKind::Defer(inner) => visit_expr(inner, rules, ctx, out),
		ExprKind::Record(fields) => {
			for (_, value) in fields {
				visit_expr(value, rules, ctx, out);
			}
		}
		ExprKind::RecordUpdate { base, fields } => {
			visit_expr(base, rules, ctx, out);
			for (_, value) in fields {
				visit_expr(value, rules, ctx, out);
			}
		}
		ExprKind::Tuple(entries) => visit_each(entries, rules, ctx, out),
		ExprKind::Try(TryNode {
			pattern,
			value,
			rest,
			..
		}) => {
			visit_expr(value, rules, ctx, out);
			// The `try` pattern binds for the continuation.
			ctx.push(Vec::new());
			bind_pattern(pattern, ctx);
			visit_body(rest, rules, ctx, out);
			ctx.pop();
		}
		ExprKind::List(items) => {
			for item in items {
				visit_expr(item.expr(), rules, ctx, out);
			}
		}
		ExprKind::If(if_node) => {
			visit_expr(&if_node.subject, rules, ctx, out);
			// An `is <pat>` binds within the then-branch.
			ctx.push(Vec::new());
			bind_pattern(&if_node.pattern, ctx);
			visit_body(&if_node.body, rules, ctx, out);
			ctx.pop();
			if let Some(else_body) = &if_node.else_body {
				visit_body(else_body, rules, ctx, out);
			}
		}
		ExprKind::When(when_node) => {
			visit_expr(&when_node.subject, rules, ctx, out);
			for case in &when_node.cases {
				ctx.push(Vec::new());
				bind_pattern(&case.pattern, ctx);
				visit_body(&case.body, rules, ctx, out);
				ctx.pop();
			}
		}
		ExprKind::While(while_node) => {
			visit_expr(&while_node.subject, rules, ctx, out);
			ctx.push(Vec::new());
			bind_pattern(&while_node.pattern, ctx);
			visit_body(&while_node.body, rules, ctx, out);
			ctx.pop();
		}
		ExprKind::Scope(scope_node) => {
			// `scope as s` binds the handle within the body.
			let handle = scope_node.handle.iter().map(|h| h.name.clone()).collect();
			ctx.push(handle);
			visit_body(&scope_node.body, rules, ctx, out);
			ctx.pop();
		}
		ExprKind::Using { body, .. } => visit_body(body, rules, ctx, out),
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

/// Bind every identifier a pattern introduces into the current scope frame.
fn bind_pattern(pattern: &PatternNode, ctx: &mut Context) {
	match &pattern.kind {
		PatternKind::Identifier(id) => ctx.bind(id.name.clone()),
		PatternKind::Constructor(_, args) => {
			for arg in args {
				bind_pattern(arg, ctx);
			}
		}
		PatternKind::Tuple(items) => {
			for item in items {
				bind_pattern(item, ctx);
			}
		}
		PatternKind::Record { fields, rest } => {
			for (_, field) in fields {
				bind_pattern(field, ctx);
			}
			if let Some(rest) = rest {
				if let Some(binding) = &rest.binding {
					ctx.bind(binding.name.clone());
				}
			}
		}
		PatternKind::List { items, rest } => {
			for item in items {
				bind_pattern(item, ctx);
			}
			if let Some(rest) = rest {
				if let Some(binding) = &rest.binding {
					ctx.bind(binding.name.clone());
				}
			}
		}
		// No bindings: wildcard, literals, interpolation matches.
		PatternKind::Underscore | PatternKind::Literal(_) | PatternKind::Interpolation(_) => {}
	}
}
