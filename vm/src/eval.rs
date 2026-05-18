// Builtin evaluation. Operator handlers are inlined directly in the VM
// dispatch loop now; what's left here is just the builtin-call path used by
// CallBuiltin / Closure-of-Builtin and the cross-call `invoke` helper.

use crate::builtin::Builtin;
use crate::value::{values_eq, Value, VariantData};
use crate::vm::{RuntimeError, VM};
use std::rc::Rc;

// Construct a prelude `option` value. `Some(payload)` for `Some(v)`, `None`
// for absent. Used by list builtins that may return no result (head, tail,
// find).
fn option_value(payload: Option<Value>) -> Value {
	let (variant, payload) = match payload {
		Some(v) => ("some", vec![v]),
		None => ("none", vec![]),
	};
	Value::Variant(Rc::new(VariantData {
		qualified_enum: Rc::new("__prelude__.option".to_string()),
		variant: Rc::new(variant.to_string()),
		payload,
	}))
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
		ListHead => {
			let xs = expect_list(&args, "head");
			Ok(option_value(xs.first().cloned()))
		}
		ListTail => {
			let xs = expect_list(&args, "tail");
			Ok(if xs.is_empty() {
				option_value(None)
			} else {
				option_value(Some(Value::List(Rc::new(xs[1..].to_vec()))))
			})
		}
		ListTake => {
			debug_assert_eq!(args.len(), 2, "`take` arity");
			let xs = match &args[0] {
				Value::List(xs) => xs.clone(),
				_ => unreachable!("`take`: expected list"),
			};
			let n = match &args[1] {
				Value::Int(n) => (*n).max(0) as usize,
				_ => unreachable!("`take`: expected int"),
			};
			let n = n.min(xs.len());
			Ok(Value::List(Rc::new(xs[..n].to_vec())))
		}
		ListDrop => {
			debug_assert_eq!(args.len(), 2, "`drop` arity");
			let xs = match &args[0] {
				Value::List(xs) => xs.clone(),
				_ => unreachable!("`drop`: expected list"),
			};
			let n = match &args[1] {
				Value::Int(n) => (*n).max(0) as usize,
				_ => unreachable!("`drop`: expected int"),
			};
			let n = n.min(xs.len());
			Ok(Value::List(Rc::new(xs[n..].to_vec())))
		}
		ListFind => {
			debug_assert_eq!(args.len(), 2, "`find` arity");
			let mut it = args.into_iter();
			let list_arg = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let xs = match list_arg {
				Value::List(xs) => xs,
				_ => unreachable!("`find`: expected list"),
			};
			for x in xs.iter() {
				match invoke(vm, fn_arg.clone(), vec![x.clone()])? {
					Value::Bool(true) => return Ok(option_value(Some(x.clone()))),
					Value::Bool(false) => {}
					_ => unreachable!("`find`: predicate must return bool"),
				}
			}
			Ok(option_value(None))
		}
		ListAny => {
			debug_assert_eq!(args.len(), 2, "`any` arity");
			let mut it = args.into_iter();
			let list_arg = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let xs = match list_arg {
				Value::List(xs) => xs,
				_ => unreachable!("`any`: expected list"),
			};
			for x in xs.iter() {
				match invoke(vm, fn_arg.clone(), vec![x.clone()])? {
					Value::Bool(true) => return Ok(Value::Bool(true)),
					Value::Bool(false) => {}
					_ => unreachable!("`any`: predicate must return bool"),
				}
			}
			Ok(Value::Bool(false))
		}
		ListAll => {
			debug_assert_eq!(args.len(), 2, "`all` arity");
			let mut it = args.into_iter();
			let list_arg = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let xs = match list_arg {
				Value::List(xs) => xs,
				_ => unreachable!("`all`: expected list"),
			};
			for x in xs.iter() {
				match invoke(vm, fn_arg.clone(), vec![x.clone()])? {
					Value::Bool(false) => return Ok(Value::Bool(false)),
					Value::Bool(true) => {}
					_ => unreachable!("`all`: predicate must return bool"),
				}
			}
			Ok(Value::Bool(true))
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
		MathFloor => Ok(Value::Int(expect_float(&args, "floor").floor() as i64)),
		MathCeil => Ok(Value::Int(expect_float(&args, "ceil").ceil() as i64)),
		MathRound => Ok(Value::Int(expect_float(&args, "round").round() as i64)),
		MathLog => Ok(Value::Float(expect_float(&args, "log").ln())),
		MathLog10 => Ok(Value::Float(expect_float(&args, "log10").log10())),
		MathLog2 => Ok(Value::Float(expect_float(&args, "log2").log2())),
		MathExp => Ok(Value::Float(expect_float(&args, "exp").exp())),
		MathSin => Ok(Value::Float(expect_float(&args, "sin").sin())),
		MathCos => Ok(Value::Float(expect_float(&args, "cos").cos())),
		MathTan => Ok(Value::Float(expect_float(&args, "tan").tan())),
		StringLength => {
			let s = expect_string(&args, "length");
			Ok(Value::Int(s.chars().count() as i64))
		}
		StringIsEmpty => {
			let s = expect_string(&args, "is-empty");
			Ok(Value::Bool(s.is_empty()))
		}
		StringToUpper => {
			let s = expect_string(&args, "to-upper");
			Ok(Value::String(Rc::new(s.to_uppercase())))
		}
		StringToLower => {
			let s = expect_string(&args, "to-lower");
			Ok(Value::String(Rc::new(s.to_lowercase())))
		}
		StringTrim => {
			let s = expect_string(&args, "trim");
			Ok(Value::String(Rc::new(s.trim().to_string())))
		}
		StringContains => {
			debug_assert_eq!(args.len(), 2, "`contains` arity");
			match (&args[0], &args[1]) {
				(Value::String(haystack), Value::String(needle)) => {
					Ok(Value::Bool(haystack.contains(needle.as_str())))
				}
				_ => unreachable!("string `contains`: expected (string, string)"),
			}
		}
		StringStartsWith => {
			debug_assert_eq!(args.len(), 2, "`starts-with` arity");
			match (&args[0], &args[1]) {
				(Value::String(s), Value::String(prefix)) => {
					Ok(Value::Bool(s.starts_with(prefix.as_str())))
				}
				_ => unreachable!("`starts-with`: expected (string, string)"),
			}
		}
		StringEndsWith => {
			debug_assert_eq!(args.len(), 2, "`ends-with` arity");
			match (&args[0], &args[1]) {
				(Value::String(s), Value::String(suffix)) => Ok(Value::Bool(s.ends_with(suffix.as_str()))),
				_ => unreachable!("`ends-with`: expected (string, string)"),
			}
		}
		StringJoin => {
			debug_assert_eq!(args.len(), 2, "`join` arity");
			let xs = match &args[0] {
				Value::List(xs) => xs,
				_ => unreachable!("`join`: expected list"),
			};
			let sep = match &args[1] {
				Value::String(s) => s,
				_ => unreachable!("`join`: expected string separator"),
			};
			let parts: Vec<&str> = xs
				.iter()
				.map(|v| match v {
					Value::String(s) => s.as_str(),
					_ => unreachable!("`join`: list element must be string"),
				})
				.collect();
			Ok(Value::String(Rc::new(parts.join(sep.as_str()))))
		}
		StringSplit => {
			debug_assert_eq!(args.len(), 2, "`split` arity");
			let s = match &args[0] {
				Value::String(s) => s,
				_ => unreachable!("`split`: expected string"),
			};
			let sep = match &args[1] {
				Value::String(s) => s,
				_ => unreachable!("`split`: expected string separator"),
			};
			// Empty separator: split into individual characters (Rust's
			// default behavior wraps with empty leading/trailing entries,
			// which is surprising for users).
			let parts: Vec<Value> = if sep.is_empty() {
				s.chars()
					.map(|c| Value::String(Rc::new(c.to_string())))
					.collect()
			} else {
				s.split(sep.as_str())
					.map(|part| Value::String(Rc::new(part.to_string())))
					.collect()
			};
			Ok(Value::List(Rc::new(parts)))
		}
		StringReplace => {
			debug_assert_eq!(args.len(), 3, "`replace` arity");
			match (&args[0], &args[1], &args[2]) {
				(Value::String(s), Value::String(from), Value::String(to)) => Ok(Value::String(Rc::new(
					s.replace(from.as_str(), to.as_str()),
				))),
				_ => unreachable!("`replace`: expected (string, string, string)"),
			}
		}
		IoPrint => {
			debug_assert_eq!(args.len(), 1, "`io.print` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stdout.write_line(&format!("{}", arg));
			Ok(arg)
		}
		IoPrintErr => {
			debug_assert_eq!(args.len(), 1, "`print-err` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stderr.write_line(&format!("{}", arg));
			Ok(arg)
		}
		IoWrite => {
			debug_assert_eq!(args.len(), 1, "`io.write` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stdout.write(&format!("{}", arg));
			Ok(arg)
		}
		IoWriteErr => {
			debug_assert_eq!(args.len(), 1, "`write-err` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stderr.write(&format!("{}", arg));
			Ok(arg)
		}
		IoRead => {
			// Called as `read ()` — lone arg is `nothing`.
			debug_assert_eq!(args.len(), 1, "`read` arity");
			Ok(match vm.stdin.read_line() {
				Ok(Some(line)) => result_ok(Value::String(Rc::new(line))),
				Ok(None) => result_err(Value::String(Rc::new("EOF".to_string()))),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		IoReadAll => {
			debug_assert_eq!(args.len(), 1, "`read-all` arity");
			Ok(match vm.stdin.read_all() {
				Ok(s) => result_ok(Value::String(Rc::new(s))),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		IoReadFile => {
			let path = expect_string(&args, "read-file");
			Ok(match std::fs::read_to_string(path.as_str()) {
				Ok(contents) => result_ok(Value::String(Rc::new(contents))),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		IoWriteFile => {
			debug_assert_eq!(args.len(), 2, "`write-file` arity");
			let (path, contents) = match (&args[0], &args[1]) {
				(Value::String(p), Value::String(c)) => (p, c),
				_ => unreachable!("`write-file`: expected (string, string)"),
			};
			Ok(match std::fs::write(path.as_str(), contents.as_bytes()) {
				Ok(()) => result_ok(Value::Nothing),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		IoAppendFile => {
			debug_assert_eq!(args.len(), 2, "`append-file` arity");
			let (path, contents) = match (&args[0], &args[1]) {
				(Value::String(p), Value::String(c)) => (p, c),
				_ => unreachable!("`append-file`: expected (string, string)"),
			};
			use std::io::Write;
			let result = std::fs::OpenOptions::new()
				.create(true)
				.append(true)
				.open(path.as_str())
				.and_then(|mut f| f.write_all(contents.as_bytes()));
			Ok(match result {
				Ok(()) => result_ok(Value::Nothing),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		IoFileExists => {
			let path = expect_string(&args, "file-exists");
			Ok(Value::Bool(std::path::Path::new(path.as_str()).exists()))
		}
		IoDeleteFile => {
			let path = expect_string(&args, "delete-file");
			Ok(match std::fs::remove_file(path.as_str()) {
				Ok(()) => result_ok(Value::Nothing),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		IoArgs => {
			// Called as `args ()` — the lone arg is the `nothing` unit.
			debug_assert_eq!(args.len(), 1, "`args` arity");
			// Skip argv[0] (the program path itself).
			let args_list: Vec<Value> = std::env::args()
				.skip(1)
				.map(|a| Value::String(Rc::new(a)))
				.collect();
			Ok(Value::List(Rc::new(args_list)))
		}
		IoEnv => {
			let name = expect_string(&args, "env");
			Ok(option_value(
				std::env::var(name.as_str())
					.ok()
					.map(|v| Value::String(Rc::new(v))),
			))
		}
		IoExit => {
			debug_assert_eq!(args.len(), 1, "`exit` arity");
			let code = match &args[0] {
				Value::Int(n) => *n as i32,
				_ => unreachable!("`exit`: expected int"),
			};
			std::process::exit(code);
		}
	}
}

// Construct a prelude `result` value. Mirrors option_value but for ok/err.
fn result_ok(payload: Value) -> Value {
	Value::Variant(Rc::new(VariantData {
		qualified_enum: Rc::new("__prelude__.result".to_string()),
		variant: Rc::new("ok".to_string()),
		payload: vec![payload],
	}))
}

fn result_err(payload: Value) -> Value {
	Value::Variant(Rc::new(VariantData {
		qualified_enum: Rc::new("__prelude__.result".to_string()),
		variant: Rc::new("err".to_string()),
		payload: vec![payload],
	}))
}

fn expect_list<'a>(args: &'a [Value], name: &str) -> &'a Rc<Vec<Value>> {
	debug_assert_eq!(args.len(), 1, "`{}` arity", name);
	match &args[0] {
		Value::List(xs) => xs,
		_ => unreachable!("`{}`: expected list", name),
	}
}

fn expect_string<'a>(args: &'a [Value], name: &str) -> &'a Rc<String> {
	debug_assert_eq!(args.len(), 1, "`{}` arity", name);
	match &args[0] {
		Value::String(s) => s,
		_ => unreachable!("`{}`: expected string", name),
	}
}

fn expect_float(args: &[Value], name: &str) -> f64 {
	debug_assert_eq!(args.len(), 1, "`{}` arity", name);
	match &args[0] {
		Value::Float(n) => *n,
		_ => unreachable!("`{}`: expected float", name),
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
			let captures = Rc::clone(&c.captures);
			let target_depth = vm.frames_len();
			vm.push_frame_with_args(fn_idx, captures, args, None)?;
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
