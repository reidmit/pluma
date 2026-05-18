// Operator and builtin evaluation, factored out of the dispatch loop. The
// dispatch loop pops the operands and pushes the result; these functions
// just compute.

use crate::builtin::Builtin;
use crate::instruction::Instruction;
use crate::value::{values_eq, Value};
use crate::vm::{RuntimeError, VM};
use std::rc::Rc;

pub fn binary(instr: &Instruction, l: Value, r: Value) -> Result<Value, RuntimeError> {
	use Instruction::*;
	match instr {
		AddInt => match (&l, &r) {
			(Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_add(*b))),
			_ => Err(RuntimeError::new("AddInt: expected ints")),
		},
		AddFloat => match (&l, &r) {
			(Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
			_ => Err(RuntimeError::new("AddFloat: expected floats")),
		},
		SubInt => match (&l, &r) {
			(Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_sub(*b))),
			_ => Err(RuntimeError::new("SubInt: expected ints")),
		},
		SubFloat => match (&l, &r) {
			(Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
			_ => Err(RuntimeError::new("SubFloat: expected floats")),
		},
		MulInt => match (&l, &r) {
			(Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_mul(*b))),
			_ => Err(RuntimeError::new("MulInt: expected ints")),
		},
		MulFloat => match (&l, &r) {
			(Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
			_ => Err(RuntimeError::new("MulFloat: expected floats")),
		},
		DivInt => match (&l, &r) {
			(Value::Int(_), Value::Int(0)) => Err(RuntimeError::new("division by zero")),
			(Value::Int(a), Value::Int(b)) => Ok(Value::Int(a / b)),
			_ => Err(RuntimeError::new("DivInt: expected ints")),
		},
		DivFloat => match (&l, &r) {
			(Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
			_ => Err(RuntimeError::new("DivFloat: expected floats")),
		},
		RemInt => match (&l, &r) {
			(Value::Int(_), Value::Int(0)) => Err(RuntimeError::new("division by zero")),
			(Value::Int(a), Value::Int(b)) => Ok(Value::Int(a % b)),
			_ => Err(RuntimeError::new("RemInt: expected ints")),
		},
		RemFloat => match (&l, &r) {
			(Value::Float(a), Value::Float(b)) => Ok(Value::Float(a % b)),
			_ => Err(RuntimeError::new("RemFloat: expected floats")),
		},
		_ => unreachable!("binary called with non-binary op"),
	}
}

pub fn unary(instr: &Instruction, v: Value) -> Result<Value, RuntimeError> {
	use Instruction::*;
	match (instr, v) {
		(NegInt, Value::Int(n)) => Ok(Value::Int(n.wrapping_neg())),
		(NegFloat, Value::Float(n)) => Ok(Value::Float(-n)),
		(NegInt, _) => Err(RuntimeError::new("NegInt: expected int")),
		(NegFloat, _) => Err(RuntimeError::new("NegFloat: expected float")),
		_ => unreachable!("unary called with non-unary op"),
	}
}

pub fn compare(instr: &Instruction, l: Value, r: Value) -> Result<Value, RuntimeError> {
	use Instruction::*;
	let ord = match (&l, &r) {
		(Value::Int(a), Value::Int(b)) => a.cmp(b),
		(Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
		_ => return Err(RuntimeError::new("ordering compare: expected numbers")),
	};
	let result = match instr {
		Lt => ord == std::cmp::Ordering::Less,
		Lte => ord != std::cmp::Ordering::Greater,
		Gt => ord == std::cmp::Ordering::Greater,
		Gte => ord != std::cmp::Ordering::Less,
		_ => unreachable!("compare called with non-compare op"),
	};
	Ok(Value::Bool(result))
}

// Arities and arg types of every builtin are statically enforced by the
// analyzer against the signatures in `stdlib.rs`. The asserts and
// `unreachable!`s below catch compiler bugs in debug builds; release builds
// trust the type system.
pub fn call_builtin(vm: &mut VM, b: Builtin, args: Vec<Value>) -> Result<Value, RuntimeError> {
	use Builtin::*;
	match b {
		Print => {
			debug_assert_eq!(args.len(), 1, "`print` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stdout.write_line(&format!("{}", arg));
			Ok(arg)
		}
		ToString => {
			debug_assert_eq!(args.len(), 1, "`to-string` arity");
			Ok(Value::String(Rc::new(format!("{}", args[0]))))
		}
		Matches => {
			debug_assert_eq!(args.len(), 2, "`matches` arity");
			match (&args[0], &args[1]) {
				(Value::Regex(re), Value::String(s)) => Ok(Value::Bool(re.compiled.is_match(s))),
				_ => unreachable!("`matches` expects (regex, string)"),
			}
		}
		ListLength => {
			let xs = expect_list(&args, "length");
			Ok(Value::Int(xs.len() as i64))
		}
		ListIsEmpty => {
			let xs = expect_list(&args, "is-empty");
			Ok(Value::Bool(xs.is_empty()))
		}
		ListReverse => {
			let xs = expect_list(&args, "reverse");
			let mut rev: Vec<Value> = xs.iter().cloned().collect();
			rev.reverse();
			Ok(Value::List(Rc::new(rev)))
		}
		ListConcat => {
			debug_assert_eq!(args.len(), 2, "`concat` arity");
			let a = match &args[0] {
				Value::List(xs) => xs.clone(),
				_ => unreachable!("`concat`: expected list"),
			};
			let b = match &args[1] {
				Value::List(xs) => xs.clone(),
				_ => unreachable!("`concat`: expected list"),
			};
			let mut out: Vec<Value> = Vec::with_capacity(a.len() + b.len());
			out.extend(a.iter().cloned());
			out.extend(b.iter().cloned());
			Ok(Value::List(Rc::new(out)))
		}
		ListContains => {
			debug_assert_eq!(args.len(), 2, "`contains` arity");
			let xs = match &args[0] {
				Value::List(xs) => xs.clone(),
				_ => unreachable!("`contains`: expected list"),
			};
			let needle = &args[1];
			Ok(Value::Bool(xs.iter().any(|v| values_eq(v, needle))))
		}
		ListMap => {
			debug_assert_eq!(args.len(), 2, "`map` arity");
			let mut it = args.into_iter();
			let list_arg = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let xs = match list_arg {
				Value::List(xs) => xs,
				_ => unreachable!("`map`: expected list"),
			};
			let mut out = Vec::with_capacity(xs.len());
			for x in xs.iter() {
				let r = invoke(vm, fn_arg.clone(), vec![x.clone()])?;
				out.push(r);
			}
			Ok(Value::List(Rc::new(out)))
		}
		ListFilter => {
			debug_assert_eq!(args.len(), 2, "`filter` arity");
			let mut it = args.into_iter();
			let list_arg = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let xs = match list_arg {
				Value::List(xs) => xs,
				_ => unreachable!("`filter`: expected list"),
			};
			let mut out = Vec::new();
			for x in xs.iter() {
				let keep = invoke(vm, fn_arg.clone(), vec![x.clone()])?;
				match keep {
					Value::Bool(true) => out.push(x.clone()),
					Value::Bool(false) => {}
					_ => unreachable!("`filter`: predicate must return bool"),
				}
			}
			Ok(Value::List(Rc::new(out)))
		}
		ListFold => {
			debug_assert_eq!(args.len(), 3, "`fold` arity");
			let mut it = args.into_iter();
			let list_arg = it.next().unwrap();
			let mut acc = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let xs = match list_arg {
				Value::List(xs) => xs,
				_ => unreachable!("`fold`: expected list"),
			};
			for x in xs.iter() {
				acc = invoke(vm, fn_arg.clone(), vec![acc, x.clone()])?;
			}
			Ok(acc)
		}
		ListEach => {
			debug_assert_eq!(args.len(), 2, "`each` arity");
			let mut it = args.into_iter();
			let list_arg = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let xs = match list_arg {
				Value::List(xs) => xs,
				_ => unreachable!("`each`: expected list"),
			};
			for x in xs.iter() {
				invoke(vm, fn_arg.clone(), vec![x.clone()])?;
			}
			Ok(Value::Nothing)
		}
		MathToFloat => {
			debug_assert_eq!(args.len(), 1, "`to-float` arity");
			match &args[0] {
				Value::Int(n) => Ok(Value::Float(*n as f64)),
				_ => unreachable!("`to-float`: expected int"),
			}
		}
		MathToInt => {
			debug_assert_eq!(args.len(), 1, "`to-int` arity");
			match &args[0] {
				Value::Float(n) => Ok(Value::Int(*n as i64)),
				_ => unreachable!("`to-int`: expected float"),
			}
		}
		MathSqrt => {
			debug_assert_eq!(args.len(), 1, "`sqrt` arity");
			match &args[0] {
				Value::Float(n) => Ok(Value::Float(n.sqrt())),
				_ => unreachable!("`sqrt`: expected float"),
			}
		}
		MathAbs => {
			debug_assert_eq!(args.len(), 1, "`abs` arity");
			match &args[0] {
				Value::Int(n) => Ok(Value::Int(n.wrapping_abs())),
				_ => unreachable!("`abs`: expected int"),
			}
		}
	}
}

fn expect_list<'a>(args: &'a [Value], name: &str) -> &'a Rc<Vec<Value>> {
	debug_assert_eq!(args.len(), 1, "`{}` arity", name);
	match &args[0] {
		Value::List(xs) => xs,
		_ => unreachable!("`{}`: expected list", name),
	}
}

// Invoke a callable (Closure / Builtin / VariantCtor) and return its result.
// Used by builtins that need to call user-supplied closures (map, filter,
// fold, each). Re-enters the VM dispatch loop on a nested basis by pushing
// the closure's frame and running until the depth returns to before.
fn invoke(vm: &mut VM, callee: Value, args: Vec<Value>) -> Result<Value, RuntimeError> {
	match callee {
		Value::Closure(c) => {
			let fn_idx = c.fn_idx as u32;
			let captures = Rc::new(c.captures.clone());
			let target_depth = vm.frames_len();
			vm.push_frame(fn_idx, captures, args, None)?;
			vm.run_until_frame_depth(target_depth)?;
			vm.pop_stack()
				.ok_or_else(|| RuntimeError::new("VM: invoke: closure returned with empty stack"))
		}
		Value::Builtin(b) => call_builtin(vm, b, args),
		Value::VariantCtor(c) => {
			debug_assert_eq!(args.len(), c.arity, "variant ctor arity");
			Ok(Value::Variant(Rc::new(crate::value::VariantData {
				qualified_enum: c.qualified_enum.clone(),
				variant: c.variant.clone(),
				payload: args,
			})))
		}
		_ => unreachable!("invoke: callee is not callable"),
	}
}
