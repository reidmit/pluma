use crate::env::Environment;
use crate::eval::pattern::match_pattern;
use crate::interpreter::{apply, Interpreter, RuntimeError};
use crate::value::Value;
use compiler::ast::{
	CallNode, ExprKind, ExprNode, FunNode, IfNode, LetNode, LiteralKind, Operator, WhenNode,
	WhileNode,
};
use std::collections::HashMap;
use std::rc::Rc;

pub fn eval_expr<'ast>(
	interp: &Interpreter<'ast>,
	env: &mut Environment<'ast>,
	current_module: &str,
	expr: &'ast ExprNode,
) -> Result<Value<'ast>, RuntimeError> {
	match &expr.kind {
		ExprKind::Literal(lit) => Ok(eval_literal(&lit.kind)),

		ExprKind::EmptyTuple => Ok(Value::Nothing),

		ExprKind::Identifier(ident) => {
			if let Some(v) = env.lookup(&ident.name) {
				return Ok(v.clone());
			}
			// Fall back to the current module's top-level defs.
			interp.force_top(current_module, &ident.name)
		}

		ExprKind::Grouping(inner) => eval_expr(interp, env, current_module, inner),

		ExprKind::Tuple(elems) => {
			let mut values = Vec::with_capacity(elems.len());
			for e in elems {
				values.push(eval_expr(interp, env, current_module, e)?);
			}
			Ok(Value::Tuple(values))
		}

		ExprKind::Record(fields) => {
			let mut map = HashMap::with_capacity(fields.len());
			for (name, value_expr) in fields {
				let v = eval_expr(interp, env, current_module, value_expr)?;
				map.insert(name.name.clone(), v);
			}
			Ok(Value::Record(Rc::new(map)))
		}

		ExprKind::Interpolation(parts) => {
			let mut out = String::new();
			for part in parts {
				let v = eval_expr(interp, env, current_module, part)?;
				match v {
					Value::String(s) => out.push_str(&s),
					other => out.push_str(&format!("{}", other)),
				}
			}
			Ok(Value::String(Rc::new(out)))
		}

		ExprKind::Let(LetNode { name, value, .. }) => {
			let v = eval_expr(interp, env, current_module, value)?;
			env.define(name.name.clone(), v);
			Ok(Value::Nothing)
		}

		ExprKind::Fun(FunNode { params, body, .. }) => {
			let param_names = params.iter().map(|p| p.ident.name.clone()).collect();
			Ok(Value::Closure {
				params: param_names,
				body,
				env: env.clone(),
				defining_module: current_module.to_string(),
			})
		}

		ExprKind::Call(CallNode { callee, args, .. }) => {
			let callee_value = eval_expr(interp, env, current_module, callee)?;
			let mut arg_values = Vec::with_capacity(args.len());
			for a in args {
				arg_values.push(eval_expr(interp, env, current_module, a)?);
			}
			apply(interp, callee_value, arg_values, expr.range, current_module)
		}

		ExprKind::FieldAccess { receiver, field } => {
			// Chained `module.enum-name.variant`: the receiver is itself a
			// FieldAccess of (module ident, enum field).
			if let ExprKind::FieldAccess {
				receiver: outer,
				field: enum_field,
			} = &receiver.kind
			{
				if let ExprKind::Identifier(module_ident) = &outer.kind {
					if let Some(qualified_module) =
						resolve_module(interp, current_module, &module_ident.name)
					{
						if let Some(variants) = interp
							.module_tops
							.get(&qualified_module)
							.and_then(|t| t.enums.get(&enum_field.name))
						{
							if let Some((_, arity)) =
								variants.iter().find(|(n, _)| n == &field.name)
							{
								let qualified_enum =
									format!("{}.{}", qualified_module, enum_field.name);
								if *arity == 0 {
									return Ok(Value::Variant {
										qualified_enum,
										variant: field.name.clone(),
										payload: vec![],
									});
								}
								return Ok(Value::VariantCtor {
									qualified_enum,
									variant: field.name.clone(),
									arity: *arity,
								});
							}
						}
					}
				}
			}

			// `module.value` access — resolve into the imported module's top
			// level.
			if let ExprKind::Identifier(ident) = &receiver.kind {
				if let Some(qualified_module) =
					resolve_module(interp, current_module, &ident.name)
				{
					if interp
						.module_tops
						.get(&qualified_module)
						.map_or(false, |t| t.has_def(&field.name))
					{
						return interp.force_top(&qualified_module, &field.name);
					}
				}
			}

			// `enum-name.variant` (same module): the receiver is a bare ident
			// matching an enum in the current module. Resolve to a Variant
			// value (zero-arg) or a VariantCtor (with payload).
			if let ExprKind::Identifier(ident) = &receiver.kind {
				if let Some(tops) = interp.module_tops.get(current_module) {
					if let Some(variants) = tops.enums.get(&ident.name) {
						if let Some((_, arity)) = variants.iter().find(|(n, _)| n == &field.name) {
							let qualified = format!("{}.{}", current_module, ident.name);
							if *arity == 0 {
								return Ok(Value::Variant {
									qualified_enum: qualified,
									variant: field.name.clone(),
									payload: vec![],
								});
							}
							return Ok(Value::VariantCtor {
								qualified_enum: qualified,
								variant: field.name.clone(),
								arity: *arity,
							});
						}
					}
				}
			}

			let recv = eval_expr(interp, env, current_module, receiver)?;
			match recv {
				Value::Record(fields) => fields
					.get(&field.name)
					.cloned()
					.ok_or_else(|| {
						RuntimeError::new(format!("no field `{}` on record", field.name))
							.at(field.range)
					}),
				_ => Err(RuntimeError::new(format!(
					"field access `.{}` on non-record value",
					field.name
				))
				.at(field.range)),
			}
		}

		ExprKind::BinaryOperation { op, left, right } => {
			let l = eval_expr(interp, env, current_module, left)?;
			let r = eval_expr(interp, env, current_module, right)?;
			eval_binary(&op.kind, l, r, expr.range)
		}

		ExprKind::If(IfNode {
			subject,
			pattern,
			body,
			..
		}) => {
			let subject_value = eval_expr(interp, env, current_module, subject)?;
			env.enter_scope();
			let matched = match_pattern(interp, &subject_value, pattern, env);
			if matched {
				let mut last = Value::Nothing;
				for e in body {
					last = eval_expr(interp, env, current_module, e)?;
				}
				let _ = last;
			}
			env.leave_scope();
			// `if` always evaluates to nothing; non-matching cases silently
			// skip per the language design.
			Ok(Value::Nothing)
		}

		ExprKind::When(WhenNode { subject, cases, .. }) => {
			let subject_value = eval_expr(interp, env, current_module, subject)?;
			for case in cases {
				env.enter_scope();
				if match_pattern(interp, &subject_value, &case.pattern, env) {
					let mut last = Value::Nothing;
					for e in &case.body {
						last = eval_expr(interp, env, current_module, e)?;
					}
					env.leave_scope();
					return Ok(last);
				}
				env.leave_scope();
			}
			// The analyzer enforces exhaustiveness, so reaching here means a
			// non-exhaustive subject type slipped through (e.g. int, string).
			Err(RuntimeError::new("`when` had no matching case").at(expr.range))
		}

		ExprKind::While(WhileNode {
			subject,
			pattern,
			body,
			..
		}) => {
			loop {
				let subject_value = eval_expr(interp, env, current_module, subject)?;
				env.enter_scope();
				let matched = match_pattern(interp, &subject_value, pattern, env);
				if !matched {
					env.leave_scope();
					break;
				}
				for e in body {
					eval_expr(interp, env, current_module, e)?;
				}
				env.leave_scope();
			}
			Ok(Value::Nothing)
		}

		ExprKind::List(elems) => {
			let mut values = Vec::with_capacity(elems.len());
			for e in elems {
				values.push(eval_expr(interp, env, current_module, e)?);
			}
			Ok(Value::List(Rc::new(values)))
		}

		ExprKind::Regex(node) => Ok(Value::Regex(node)),

		// Not implemented yet.
		ExprKind::UnaryOperation { .. } | ExprKind::ElementAccess { .. } => Err(
			RuntimeError::new("interpreter does not yet handle this expression form")
				.at(expr.range),
		),
	}
}

// Resolve a bare ident to a fully-qualified module name, looking it up in the
// current module's import map. Falls back to the bare name if a module by that
// exact name exists (covers the no-`use` chained case, which the analyzer
// allows for already-loaded modules).
fn resolve_module<'ast>(
	interp: &Interpreter<'ast>,
	current_module: &str,
	name: &str,
) -> Option<String> {
	interp
		.module_tops
		.get(current_module)
		.and_then(|t| t.imports.get(name).cloned())
		.or_else(|| {
			if interp.module_tops.contains_key(name) {
				Some(name.to_string())
			} else {
				None
			}
		})
}

fn eval_literal<'ast>(kind: &LiteralKind) -> Value<'ast> {
	match kind {
		LiteralKind::Bool(b) => Value::Bool(*b),
		LiteralKind::String(s) => Value::String(Rc::new(s.clone())),
		LiteralKind::FloatDecimal(f) => Value::Float(*f),
		LiteralKind::IntDecimal(n)
		| LiteralKind::IntHex(n)
		| LiteralKind::IntOctal(n)
		| LiteralKind::IntBinary(n) => Value::Int(*n as i64),
	}
}

fn eval_binary<'ast>(
	op: &Operator,
	left: Value<'ast>,
	right: Value<'ast>,
	range: compiler::Range,
) -> Result<Value<'ast>, RuntimeError> {
	use Operator::*;
	let int_op =
		|l: &Value<'ast>, r: &Value<'ast>| match (l, r) {
			(Value::Int(a), Value::Int(b)) => Some((*a, *b)),
			_ => None,
		};
	let bool_op = |l: &Value<'ast>, r: &Value<'ast>| match (l, r) {
		(Value::Bool(a), Value::Bool(b)) => Some((*a, *b)),
		_ => None,
	};
	match op {
		Addition => int_op(&left, &right)
			.map(|(a, b)| Value::Int(a.wrapping_add(b)))
			.ok_or_else(|| RuntimeError::new("expected ints for `+`").at(range)),
		SubtractionOrNegation => int_op(&left, &right)
			.map(|(a, b)| Value::Int(a.wrapping_sub(b)))
			.ok_or_else(|| RuntimeError::new("expected ints for `-`").at(range)),
		Multiplication => int_op(&left, &right)
			.map(|(a, b)| Value::Int(a.wrapping_mul(b)))
			.ok_or_else(|| RuntimeError::new("expected ints for `*`").at(range)),
		Division => {
			let (a, b) = int_op(&left, &right)
				.ok_or_else(|| RuntimeError::new("expected ints for `/`").at(range))?;
			if b == 0 {
				return Err(RuntimeError::new("division by zero").at(range));
			}
			Ok(Value::Int(a / b))
		}
		Remainder => {
			let (a, b) = int_op(&left, &right)
				.ok_or_else(|| RuntimeError::new("expected ints for `%`").at(range))?;
			if b == 0 {
				return Err(RuntimeError::new("division by zero").at(range));
			}
			Ok(Value::Int(a % b))
		}
		LogicalAnd => bool_op(&left, &right)
			.map(|(a, b)| Value::Bool(a && b))
			.ok_or_else(|| RuntimeError::new("expected bools for `&&`").at(range)),
		LogicalOr => bool_op(&left, &right)
			.map(|(a, b)| Value::Bool(a || b))
			.ok_or_else(|| RuntimeError::new("expected bools for `||`").at(range)),
		other => Err(RuntimeError::new(format!(
			"binary operator `{}` not yet implemented",
			other
		))
		.at(range)),
	}
}
