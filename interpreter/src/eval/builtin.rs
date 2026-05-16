use crate::interpreter::RuntimeError;
use crate::value::{Builtin, Value};
use compiler::Range;
use std::rc::Rc;

pub fn call<'ast>(
	b: Builtin,
	args: Vec<Value<'ast>>,
	call_range: Range,
) -> Result<Value<'ast>, RuntimeError> {
	match b {
		Builtin::Print => print(args, call_range),
		Builtin::ToString => to_string(args, call_range),
	}
}

fn print<'ast>(args: Vec<Value<'ast>>, call_range: Range) -> Result<Value<'ast>, RuntimeError> {
	if args.len() != 1 {
		return Err(RuntimeError::new(format!(
			"`print` takes 1 argument, got {}",
			args.len()
		))
		.at(call_range));
	}
	match &args[0] {
		Value::String(s) => {
			println!("{}", s);
			Ok(Value::Nothing)
		}
		_ => Err(RuntimeError::new("`print` expected a string").at(call_range)),
	}
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
