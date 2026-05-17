use crate::eval::expr::values_eq;
use crate::interpreter::{apply, Interpreter, RuntimeError};
use crate::value::{Builtin, Value};
use compiler::Range;
use std::rc::Rc;

// Builtins that call user-supplied closures pass this as the `current_module`
// to `apply`. The argument is only meaningful for resolving free names in a
// closure body, but closures carry their own `defining_module` and always use
// that — so this value is effectively unused.
const STDLIB_CURRENT_MODULE: &str = "core.list";

pub fn call<'ast>(
	interp: &Interpreter<'ast>,
	b: Builtin,
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	match b {
		Builtin::Print => print(interp, args, call_range),
		Builtin::ToString => to_string(args, call_range),
		Builtin::Matches => matches(args, call_range),
		Builtin::ListLength => list_length(args, call_range),
		Builtin::ListIsEmpty => list_is_empty(args, call_range),
		Builtin::ListReverse => list_reverse(args, call_range),
		Builtin::ListConcat => list_concat(args, call_range),
		Builtin::ListContains => list_contains(args, call_range),
		Builtin::ListMap => list_map(interp, args, call_range),
		Builtin::ListFilter => list_filter(interp, args, call_range),
		Builtin::ListFold => list_fold(interp, args, call_range),
		Builtin::ListEach => list_each(interp, args, call_range),
	}
}

fn print<'ast>(
	interp: &Interpreter<'ast>,
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 1 {
		return Err(RuntimeError::new(format!(
			"`print` takes 1 argument, got {}",
			args.len()
		))
		.at(call_range));
	}
	let arg = args.into_iter().next().unwrap();
	interp.stdout.write_line(&format!("{}", arg));
	Ok(arg)
}

fn to_string<'ast>(
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 1 {
		return Err(RuntimeError::new(format!(
			"`to-string` takes 1 argument, got {}",
			args.len()
		))
		.at(call_range));
	}
	let rendered = format!("{}", args[0]);
	Ok(Value::String(Rc::new(rendered)))
}

fn matches<'ast>(
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 2 {
		return Err(RuntimeError::new(format!(
			"`matches` takes 2 arguments (regex, string), got {}",
			args.len()
		))
		.at(call_range));
	}
	match (&args[0], &args[1]) {
		(Value::Regex(re), Value::String(s)) => Ok(Value::Bool(re.is_match(s))),
		_ => Err(RuntimeError::new("`matches` expects (regex, string)").at(call_range)),
	}
}

fn expect_list<'a, 'ast>(
	v: &'a Value<'ast>,
	name: &str,
	range: Range,
) -> Result<&'a Rc<Vec<Value<'ast>>>, RuntimeError> {
	match v {
		Value::List(xs) => Ok(xs),
		_ => Err(RuntimeError::new(format!("`{}` expected a list", name)).at(range)),
	}
}

fn list_length<'ast>(
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 1 {
		return Err(RuntimeError::new("`length` takes 1 argument").at(call_range));
	}
	let xs = expect_list(&args[0], "length", call_range)?;
	Ok(Value::Int(xs.len() as i64))
}

fn list_is_empty<'ast>(
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 1 {
		return Err(RuntimeError::new("`is-empty` takes 1 argument").at(call_range));
	}
	let xs = expect_list(&args[0], "is-empty", call_range)?;
	Ok(Value::Bool(xs.is_empty()))
}

fn list_reverse<'ast>(
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 1 {
		return Err(RuntimeError::new("`reverse` takes 1 argument").at(call_range));
	}
	let xs = expect_list(&args[0], "reverse", call_range)?;
	let mut rev: Vec<Value<'ast>> = xs.iter().cloned().collect();
	rev.reverse();
	Ok(Value::List(Rc::new(rev)))
}

fn list_concat<'ast>(
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 2 {
		return Err(RuntimeError::new("`concat` takes 2 arguments").at(call_range));
	}
	let a = expect_list(&args[0], "concat", call_range)?;
	let b = expect_list(&args[1], "concat", call_range)?;
	let mut out: Vec<Value<'ast>> = Vec::with_capacity(a.len() + b.len());
	out.extend(a.iter().cloned());
	out.extend(b.iter().cloned());
	Ok(Value::List(Rc::new(out)))
}

fn list_contains<'ast>(
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 2 {
		return Err(RuntimeError::new("`contains` takes 2 arguments").at(call_range));
	}
	let xs = expect_list(&args[0], "contains", call_range)?;
	let needle = &args[1];
	Ok(Value::Bool(xs.iter().any(|v| values_eq(v, needle))))
}

fn list_map<'ast>(
	interp: &Interpreter<'ast>,
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 2 {
		return Err(RuntimeError::new("`map` takes 2 arguments").at(call_range));
	}
	let mut iter = args.into_iter();
	let list_arg = iter.next().unwrap();
	let fn_arg = iter.next().unwrap();
	let xs = match list_arg {
		Value::List(xs) => xs,
		_ => return Err(RuntimeError::new("`map` expected a list").at(call_range)),
	};
	let mut out = Vec::with_capacity(xs.len());
	for x in xs.iter() {
		let result = apply(
			interp,
			fn_arg.clone(),
			vec![x.clone()],
			call_range,
			STDLIB_CURRENT_MODULE,
		)?;
		out.push(result);
	}
	Ok(Value::List(Rc::new(out)))
}

fn list_filter<'ast>(
	interp: &Interpreter<'ast>,
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 2 {
		return Err(RuntimeError::new("`filter` takes 2 arguments").at(call_range));
	}
	let mut iter = args.into_iter();
	let list_arg = iter.next().unwrap();
	let fn_arg = iter.next().unwrap();
	let xs = match list_arg {
		Value::List(xs) => xs,
		_ => return Err(RuntimeError::new("`filter` expected a list").at(call_range)),
	};
	let mut out = Vec::new();
	for x in xs.iter() {
		let keep = apply(
			interp,
			fn_arg.clone(),
			vec![x.clone()],
			call_range,
			STDLIB_CURRENT_MODULE,
		)?;
		match keep {
			Value::Bool(true) => out.push(x.clone()),
			Value::Bool(false) => {}
			_ => return Err(RuntimeError::new("`filter` predicate must return bool").at(call_range)),
		}
	}
	Ok(Value::List(Rc::new(out)))
}

fn list_fold<'ast>(
	interp: &Interpreter<'ast>,
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 3 {
		return Err(RuntimeError::new("`fold` takes 3 arguments (list, init, fn)").at(call_range));
	}
	let mut iter = args.into_iter();
	let list_arg = iter.next().unwrap();
	let mut acc = iter.next().unwrap();
	let fn_arg = iter.next().unwrap();
	let xs = match list_arg {
		Value::List(xs) => xs,
		_ => return Err(RuntimeError::new("`fold` expected a list").at(call_range)),
	};
	for x in xs.iter() {
		acc = apply(
			interp,
			fn_arg.clone(),
			vec![acc, x.clone()],
			call_range,
			STDLIB_CURRENT_MODULE,
		)?;
	}
	Ok(acc)
}

fn list_each<'ast>(
	interp: &Interpreter<'ast>,
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 2 {
		return Err(RuntimeError::new("`each` takes 2 arguments").at(call_range));
	}
	let mut iter = args.into_iter();
	let list_arg = iter.next().unwrap();
	let fn_arg = iter.next().unwrap();
	let xs = match list_arg {
		Value::List(xs) => xs,
		_ => return Err(RuntimeError::new("`each` expected a list").at(call_range)),
	};
	for x in xs.iter() {
		apply(
			interp,
			fn_arg.clone(),
			vec![x.clone()],
			call_range,
			STDLIB_CURRENT_MODULE,
		)?;
	}
	Ok(Value::Nothing)
}
