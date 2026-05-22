// Builtin dispatch. Each `Value::Builtin(tag)` lands here when invoked —
// `call_builtin` matches the tag against this file's arms and runs the
// corresponding Rust implementation. Operator handlers (arithmetic,
// comparison, etc.) are inlined into the VM dispatch loop instead; this
// file is only the named-builtin path plus the cross-call `invoke` helper.

use crate::value::{values_eq, MapData, Value, VariantData};
use crate::vm::{RuntimeError, VM};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// Construct a prelude `ordering` variant from a `std::cmp::Ordering`.
// Used by the `ord` trait's int/float/string `compare` builtins.
fn ordering_variant(o: std::cmp::Ordering) -> Value {
	let variant = match o {
		std::cmp::Ordering::Less => "lt",
		std::cmp::Ordering::Equal => "eq",
		std::cmp::Ordering::Greater => "gt",
	};
	Value::Variant(Rc::new(VariantData {
		qualified_enum: Rc::new("__prelude__.ordering".to_string()),
		variant: Rc::new(variant.to_string()),
		payload: vec![],
	}))
}

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
pub fn call_builtin(vm: &mut VM, tag: &str, args: Vec<Value>) -> Result<Value, RuntimeError> {
	match tag {
		"print" => {
			debug_assert_eq!(args.len(), 1, "`print` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stdout.write_line(&format!("{}", arg));
			Ok(arg)
		}
		"to-string" => {
			debug_assert_eq!(args.len(), 1, "`to-string` arity");
			Ok(Value::String(Rc::new(format!("{}", args[0]))))
		}
		"regex-matches" => {
			debug_assert_eq!(args.len(), 2, "`matches` arity");
			match (&args[0], &args[1]) {
				(Value::Regex(re), Value::String(s)) => Ok(Value::Bool(re.compiled.is_match(s))),
				_ => unreachable!("`matches` expects (regex, string)"),
			}
		}
		"regex-find" => {
			debug_assert_eq!(args.len(), 2, "`find` arity");
			match (&args[0], &args[1]) {
				(Value::Regex(re), Value::String(s)) => match re.compiled.captures(s) {
					Some(caps) => Ok(option_value(Some(regex_match_record(&re.compiled, &caps)))),
					None => Ok(option_value(None)),
				},
				_ => unreachable!("`find` expects (regex, string)"),
			}
		}
		"regex-find-all" => {
			debug_assert_eq!(args.len(), 2, "`find-all` arity");
			match (&args[0], &args[1]) {
				(Value::Regex(re), Value::String(s)) => {
					let xs: Vec<Value> = re
						.compiled
						.captures_iter(s)
						.map(|caps| regex_match_record(&re.compiled, &caps))
						.collect();
					Ok(Value::List(Rc::new(xs)))
				}
				_ => unreachable!("`find-all` expects (regex, string)"),
			}
		}
		"regex-named-capture" => {
			debug_assert_eq!(args.len(), 3, "`named-capture` arity");
			match (&args[0], &args[1], &args[2]) {
				(Value::Regex(re), Value::String(s), Value::String(name)) => {
					let payload = re
						.compiled
						.captures(s)
						.and_then(|c| c.name(name).map(|m| m.as_str().to_string()))
						.map(|s| Value::String(Rc::new(s)));
					Ok(option_value(payload))
				}
				_ => unreachable!("`named-capture` expects (regex, string, string)"),
			}
		}
		"regex-replace" => {
			debug_assert_eq!(args.len(), 3, "`replace` arity");
			match (&args[0], &args[1], &args[2]) {
				(Value::Regex(re), Value::String(s), Value::String(rep)) => Ok(Value::String(
					Rc::new(re.compiled.replace_all(s, rep.as_str()).into_owned()),
				)),
				_ => unreachable!("`replace` expects (regex, string, string)"),
			}
		}
		"regex-replace-first" => {
			debug_assert_eq!(args.len(), 3, "`replace-first` arity");
			match (&args[0], &args[1], &args[2]) {
				(Value::Regex(re), Value::String(s), Value::String(rep)) => Ok(Value::String(
					Rc::new(re.compiled.replace(s, rep.as_str()).into_owned()),
				)),
				_ => unreachable!("`replace-first` expects (regex, string, string)"),
			}
		}
		"regex-split" => {
			debug_assert_eq!(args.len(), 2, "`split` arity");
			match (&args[0], &args[1]) {
				(Value::Regex(re), Value::String(s)) => {
					let xs: Vec<Value> = re
						.compiled
						.split(s)
						.map(|piece| Value::String(Rc::new(piece.to_string())))
						.collect();
					Ok(Value::List(Rc::new(xs)))
				}
				_ => unreachable!("`split` expects (regex, string)"),
			}
		}
		"int-add" => match (&args[0], &args[1]) {
			(Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_add(*b))),
			_ => unreachable!("`int-add` expects (int, int)"),
		},
		"int-sub" => match (&args[0], &args[1]) {
			(Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_sub(*b))),
			_ => unreachable!("`int-sub` expects (int, int)"),
		},
		"int-mul" => match (&args[0], &args[1]) {
			(Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_mul(*b))),
			_ => unreachable!("`int-mul` expects (int, int)"),
		},
		"int-div" => match (&args[0], &args[1]) {
			(Value::Int(_), Value::Int(0)) => Err(RuntimeError::new("integer division by zero")),
			(Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_div(*b))),
			_ => unreachable!("`int-div` expects (int, int)"),
		},
		"int-negate" => match &args[0] {
			Value::Int(a) => Ok(Value::Int(a.wrapping_neg())),
			_ => unreachable!("`int-negate` expects int"),
		},
		"float-add" => match (&args[0], &args[1]) {
			(Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
			_ => unreachable!("`float-add` expects (float, float)"),
		},
		"float-sub" => match (&args[0], &args[1]) {
			(Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
			_ => unreachable!("`float-sub` expects (float, float)"),
		},
		"float-mul" => match (&args[0], &args[1]) {
			(Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
			_ => unreachable!("`float-mul` expects (float, float)"),
		},
		"float-div" => match (&args[0], &args[1]) {
			(Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
			_ => unreachable!("`float-div` expects (float, float)"),
		},
		"float-negate" => match &args[0] {
			Value::Float(a) => Ok(Value::Float(-a)),
			_ => unreachable!("`float-negate` expects float"),
		},
		"int-compare" => match (&args[0], &args[1]) {
			(Value::Int(a), Value::Int(b)) => Ok(ordering_variant(a.cmp(b))),
			_ => unreachable!("`int-compare` expects (int, int)"),
		},
		"float-compare" => match (&args[0], &args[1]) {
			(Value::Float(a), Value::Float(b)) => Ok(ordering_variant(
				a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
			)),
			_ => unreachable!("`float-compare` expects (float, float)"),
		},
		"string-compare" => match (&args[0], &args[1]) {
			(Value::String(a), Value::String(b)) => Ok(ordering_variant(a.as_str().cmp(b.as_str()))),
			_ => unreachable!("`string-compare` expects (string, string)"),
		},
		"int-hash" => match &args[0] {
			Value::Int(n) => Ok(Value::Int(*n)),
			_ => unreachable!("`int-hash` expects int"),
		},
		"float-hash" => match &args[0] {
			// Reinterpret the float's bit pattern as i64. Stable across runs
			// for the same value; collisions only on bit-equal floats.
			Value::Float(f) => Ok(Value::Int(f.to_bits() as i64)),
			_ => unreachable!("`float-hash` expects float"),
		},
		"string-hash" => match &args[0] {
			Value::String(s) => {
				use std::hash::{Hash, Hasher};
				let mut h = std::collections::hash_map::DefaultHasher::new();
				s.as_str().hash(&mut h);
				Ok(Value::Int(h.finish() as i64))
			}
			_ => unreachable!("`string-hash` expects string"),
		},
		"bool-hash" => match &args[0] {
			Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
			_ => unreachable!("`bool-hash` expects bool"),
		},
		"list-length" => {
			let xs = expect_list(&args, "length");
			Ok(Value::Int(xs.len() as i64))
		}
		"list-is-empty" => {
			let xs = expect_list(&args, "is-empty");
			Ok(Value::Bool(xs.is_empty()))
		}
		"list-reverse" => {
			let xs = expect_list(&args, "reverse");
			let mut rev: Vec<Value> = xs.iter().cloned().collect();
			rev.reverse();
			Ok(Value::List(Rc::new(rev)))
		}
		"list-concat" => {
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
		"list-contains" => {
			debug_assert_eq!(args.len(), 2, "`contains` arity");
			let xs = match &args[0] {
				Value::List(xs) => xs.clone(),
				_ => unreachable!("`contains`: expected list"),
			};
			let needle = &args[1];
			Ok(Value::Bool(xs.iter().any(|v| values_eq(v, needle))))
		}
		"list-map" => {
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
		"list-filter" => {
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
		"list-fold" => {
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
		"list-each" => {
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
		"list-head" => {
			let xs = expect_list(&args, "head");
			Ok(option_value(xs.first().cloned()))
		}
		"list-tail" => {
			let xs = expect_list(&args, "tail");
			Ok(if xs.is_empty() {
				option_value(None)
			} else {
				option_value(Some(Value::List(Rc::new(xs[1..].to_vec()))))
			})
		}
		"list-take" => {
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
		"list-drop" => {
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
		"list-find" => {
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
		"list-any" => {
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
		"list-all" => {
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
		"list-sort" => {
			debug_assert_eq!(args.len(), 2, "`sort` arity");
			let mut it = args.into_iter();
			let list_arg = it.next().unwrap();
			let cmp_fn = it.next().unwrap();
			let xs = match list_arg {
				Value::List(xs) => xs,
				_ => unreachable!("`sort`: expected list"),
			};
			// Pull into a Vec we own so we can sort. The comparator returns
			// an `ordering` variant; we map it to `std::cmp::Ordering`.
			let mut out: Vec<Value> = xs.iter().cloned().collect();
			// Track the first error from the comparator so we can return
			// it after the (in-progress) sort completes. `sort_by` needs an
			// infallible `Ord` so we treat errors as `Equal` and bubble.
			let mut err: Option<RuntimeError> = None;
			out.sort_by(|a, b| {
				if err.is_some() {
					return std::cmp::Ordering::Equal;
				}
				match invoke(vm, cmp_fn.clone(), vec![a.clone(), b.clone()]) {
					Ok(Value::Variant(v)) => match v.variant.as_str() {
						"lt" => std::cmp::Ordering::Less,
						"eq" => std::cmp::Ordering::Equal,
						"gt" => std::cmp::Ordering::Greater,
						other => {
							err = Some(RuntimeError::new(format!(
								"`sort`: comparator returned `{}`; expected `lt`, `eq`, or `gt`",
								other
							)));
							std::cmp::Ordering::Equal
						}
					},
					Ok(other) => {
						err = Some(RuntimeError::new(format!(
							"`sort`: comparator returned `{}`; expected an `ordering` variant",
							other
						)));
						std::cmp::Ordering::Equal
					}
					Err(e) => {
						err = Some(e);
						std::cmp::Ordering::Equal
					}
				}
			});
			if let Some(e) = err {
				return Err(e);
			}
			Ok(Value::List(Rc::new(out)))
		}
		"math-to-float" => {
			debug_assert_eq!(args.len(), 1, "`to-float` arity");
			match &args[0] {
				Value::Int(n) => Ok(Value::Float(*n as f64)),
				_ => unreachable!("`to-float`: expected int"),
			}
		}
		"math-to-int" => {
			debug_assert_eq!(args.len(), 1, "`to-int` arity");
			match &args[0] {
				Value::Float(n) => Ok(Value::Int(*n as i64)),
				_ => unreachable!("`to-int`: expected float"),
			}
		}
		"math-sqrt" => {
			debug_assert_eq!(args.len(), 1, "`sqrt` arity");
			match &args[0] {
				Value::Float(n) => Ok(Value::Float(n.sqrt())),
				_ => unreachable!("`sqrt`: expected float"),
			}
		}
		"math-abs" => {
			debug_assert_eq!(args.len(), 1, "`abs` arity");
			match &args[0] {
				Value::Int(n) => Ok(Value::Int(n.wrapping_abs())),
				_ => unreachable!("`abs`: expected int"),
			}
		}
		"math-floor" => Ok(Value::Int(expect_float(&args, "floor").floor() as i64)),
		"math-ceil" => Ok(Value::Int(expect_float(&args, "ceil").ceil() as i64)),
		"math-round" => Ok(Value::Int(expect_float(&args, "round").round() as i64)),
		"math-log" => Ok(Value::Float(expect_float(&args, "log").ln())),
		"math-log10" => Ok(Value::Float(expect_float(&args, "log10").log10())),
		"math-log2" => Ok(Value::Float(expect_float(&args, "log2").log2())),
		"math-exp" => Ok(Value::Float(expect_float(&args, "exp").exp())),
		"math-sin" => Ok(Value::Float(expect_float(&args, "sin").sin())),
		"math-cos" => Ok(Value::Float(expect_float(&args, "cos").cos())),
		"math-tan" => Ok(Value::Float(expect_float(&args, "tan").tan())),
		"string-length" => {
			let s = expect_string(&args, "length");
			Ok(Value::Int(s.chars().count() as i64))
		}
		"string-is-empty" => {
			let s = expect_string(&args, "is-empty");
			Ok(Value::Bool(s.is_empty()))
		}
		"string-to-upper" => {
			let s = expect_string(&args, "to-upper");
			Ok(Value::String(Rc::new(s.to_uppercase())))
		}
		"string-to-lower" => {
			let s = expect_string(&args, "to-lower");
			Ok(Value::String(Rc::new(s.to_lowercase())))
		}
		"string-trim" => {
			let s = expect_string(&args, "trim");
			Ok(Value::String(Rc::new(s.trim().to_string())))
		}
		"string-contains" => {
			debug_assert_eq!(args.len(), 2, "`contains` arity");
			match (&args[0], &args[1]) {
				(Value::String(haystack), Value::String(needle)) => {
					Ok(Value::Bool(haystack.contains(needle.as_str())))
				}
				_ => unreachable!("string `contains`: expected (string, string)"),
			}
		}
		"string-starts-with" => {
			debug_assert_eq!(args.len(), 2, "`starts-with` arity");
			match (&args[0], &args[1]) {
				(Value::String(s), Value::String(prefix)) => {
					Ok(Value::Bool(s.starts_with(prefix.as_str())))
				}
				_ => unreachable!("`starts-with`: expected (string, string)"),
			}
		}
		"string-ends-with" => {
			debug_assert_eq!(args.len(), 2, "`ends-with` arity");
			match (&args[0], &args[1]) {
				(Value::String(s), Value::String(suffix)) => Ok(Value::Bool(s.ends_with(suffix.as_str()))),
				_ => unreachable!("`ends-with`: expected (string, string)"),
			}
		}
		"string-join" => {
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
		"string-split" => {
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
		"string-replace" => {
			debug_assert_eq!(args.len(), 3, "`replace` arity");
			match (&args[0], &args[1], &args[2]) {
				(Value::String(s), Value::String(from), Value::String(to)) => Ok(Value::String(Rc::new(
					s.replace(from.as_str(), to.as_str()),
				))),
				_ => unreachable!("`replace`: expected (string, string, string)"),
			}
		}
		"string-to-int" => {
			let s = expect_string(&args, "to-int");
			Ok(match s.parse::<i64>() {
				Ok(n) => result_ok(Value::Int(n)),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"string-to-float" => {
			let s = expect_string(&args, "to-float");
			Ok(match s.parse::<f64>() {
				Ok(n) => result_ok(Value::Float(n)),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"string-to-bytes" => {
			let s = expect_string(&args, "to-bytes");
			Ok(Value::Bytes(Rc::new(s.as_bytes().to_vec())))
		}
		"bytes-length" => {
			let b = expect_bytes(&args, "length");
			Ok(Value::Int(b.len() as i64))
		}
		"bytes-is-empty" => {
			let b = expect_bytes(&args, "is-empty");
			Ok(Value::Bool(b.is_empty()))
		}
		"bytes-at" => {
			debug_assert_eq!(args.len(), 2, "`bytes.at` arity");
			match (&args[0], &args[1]) {
				(Value::Bytes(b), Value::Int(i)) => Ok(option_value(
					usize::try_from(*i)
						.ok()
						.and_then(|idx| b.get(idx).copied())
						.map(|byte| Value::Int(byte as i64)),
				)),
				_ => unreachable!("`bytes.at`: expected (bytes, int)"),
			}
		}
		"bytes-concat" => {
			debug_assert_eq!(args.len(), 2, "`bytes.concat` arity");
			match (&args[0], &args[1]) {
				(Value::Bytes(a), Value::Bytes(b)) => {
					let mut out = Vec::with_capacity(a.len() + b.len());
					out.extend_from_slice(a);
					out.extend_from_slice(b);
					Ok(Value::Bytes(Rc::new(out)))
				}
				_ => unreachable!("`bytes.concat`: expected (bytes, bytes)"),
			}
		}
		"bytes-slice" => {
			debug_assert_eq!(args.len(), 3, "`bytes.slice` arity");
			match (&args[0], &args[1], &args[2]) {
				(Value::Bytes(b), Value::Int(start), Value::Int(end)) => {
					// Negative or beyond-the-end indices clamp to bounds.
					// Order is preserved: end < start collapses to empty.
					let len = b.len();
					let s = (*start).max(0) as usize;
					let s = s.min(len);
					let e = (*end).max(0) as usize;
					let e = e.min(len).max(s);
					Ok(Value::Bytes(Rc::new(b[s..e].to_vec())))
				}
				_ => unreachable!("`bytes.slice`: expected (bytes, int, int)"),
			}
		}
		"bytes-contains" => {
			debug_assert_eq!(args.len(), 2, "`bytes.contains` arity");
			match (&args[0], &args[1]) {
				(Value::Bytes(haystack), Value::Bytes(needle)) => {
					Ok(Value::Bool(bytes_contains(haystack, needle)))
				}
				_ => unreachable!("`bytes.contains`: expected (bytes, bytes)"),
			}
		}
		"bytes-starts-with" => {
			debug_assert_eq!(args.len(), 2, "`bytes.starts-with` arity");
			match (&args[0], &args[1]) {
				(Value::Bytes(b), Value::Bytes(prefix)) => Ok(Value::Bool(b.starts_with(prefix))),
				_ => unreachable!("`bytes.starts-with`: expected (bytes, bytes)"),
			}
		}
		"bytes-ends-with" => {
			debug_assert_eq!(args.len(), 2, "`bytes.ends-with` arity");
			match (&args[0], &args[1]) {
				(Value::Bytes(b), Value::Bytes(suffix)) => Ok(Value::Bool(b.ends_with(suffix))),
				_ => unreachable!("`bytes.ends-with`: expected (bytes, bytes)"),
			}
		}
		"bytes-repeat" => {
			debug_assert_eq!(args.len(), 2, "`bytes.repeat` arity");
			match (&args[0], &args[1]) {
				(Value::Bytes(b), Value::Int(n)) => {
					let n = (*n).max(0) as usize;
					let mut out = Vec::with_capacity(b.len() * n);
					for _ in 0..n {
						out.extend_from_slice(b);
					}
					Ok(Value::Bytes(Rc::new(out)))
				}
				_ => unreachable!("`bytes.repeat`: expected (bytes, int)"),
			}
		}
		"bytes-reverse" => {
			let b = expect_bytes(&args, "reverse");
			let mut out = b.as_ref().clone();
			out.reverse();
			Ok(Value::Bytes(Rc::new(out)))
		}
		"bytes-to-list" => {
			let b = expect_bytes(&args, "to-list");
			let xs: Vec<Value> = b.iter().map(|&byte| Value::Int(byte as i64)).collect();
			Ok(Value::List(Rc::new(xs)))
		}
		"bytes-from-list" => {
			let xs = expect_list(&args, "from-list");
			let mut out = Vec::with_capacity(xs.len());
			for v in xs.iter() {
				match v {
					Value::Int(n) => {
						if *n < 0 || *n > 255 {
							return Ok(result_err(Value::String(Rc::new(format!(
								"byte out of range (0..256): {}",
								n
							)))));
						}
						out.push(*n as u8);
					}
					_ => unreachable!("`bytes.from-list`: list element must be int"),
				}
			}
			Ok(result_ok(Value::Bytes(Rc::new(out))))
		}
		"bytes-join" => {
			debug_assert_eq!(args.len(), 2, "`bytes.join` arity");
			let xs = match &args[0] {
				Value::List(xs) => xs,
				_ => unreachable!("`bytes.join`: expected list"),
			};
			let sep = match &args[1] {
				Value::Bytes(s) => s,
				_ => unreachable!("`bytes.join`: expected bytes separator"),
			};
			let parts: Vec<&[u8]> = xs
				.iter()
				.map(|v| match v {
					Value::Bytes(b) => b.as_slice(),
					_ => unreachable!("`bytes.join`: list element must be bytes"),
				})
				.collect();
			let mut out = Vec::new();
			for (i, p) in parts.iter().enumerate() {
				if i > 0 {
					out.extend_from_slice(sep);
				}
				out.extend_from_slice(p);
			}
			Ok(Value::Bytes(Rc::new(out)))
		}
		"bytes-split" => {
			debug_assert_eq!(args.len(), 2, "`bytes.split` arity");
			let b = match &args[0] {
				Value::Bytes(b) => b,
				_ => unreachable!("`bytes.split`: expected bytes"),
			};
			let sep = match &args[1] {
				Value::Bytes(s) => s,
				_ => unreachable!("`bytes.split`: expected bytes separator"),
			};
			let parts: Vec<Value> = if sep.is_empty() {
				// Empty separator: split into single-byte chunks. Parallel
				// to `string.split s ""`.
				b.iter()
					.map(|&byte| Value::Bytes(Rc::new(vec![byte])))
					.collect()
			} else {
				bytes_split(b, sep)
					.into_iter()
					.map(|chunk| Value::Bytes(Rc::new(chunk)))
					.collect()
			};
			Ok(Value::List(Rc::new(parts)))
		}
		"bytes-to-string" => {
			let b = expect_bytes(&args, "to-string");
			match std::str::from_utf8(b) {
				Ok(s) => Ok(result_ok(Value::String(Rc::new(s.to_string())))),
				Err(e) => Ok(result_err(Value::String(Rc::new(e.to_string())))),
			}
		}
		"bytes-compare" => match (&args[0], &args[1]) {
			(Value::Bytes(a), Value::Bytes(b)) => Ok(ordering_variant(a.as_slice().cmp(b.as_slice()))),
			_ => unreachable!("`bytes-compare` expects (bytes, bytes)"),
		},
		"bytes-hash" => match &args[0] {
			Value::Bytes(b) => {
				use std::hash::{Hash, Hasher};
				let mut h = std::collections::hash_map::DefaultHasher::new();
				b.as_slice().hash(&mut h);
				Ok(Value::Int(h.finish() as i64))
			}
			_ => unreachable!("`bytes-hash` expects bytes"),
		},
		"io-print" => {
			debug_assert_eq!(args.len(), 1, "`io.print` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stdout.write_line(&format!("{}", arg));
			Ok(arg)
		}
		"io-print-err" => {
			debug_assert_eq!(args.len(), 1, "`print-err` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stderr.write_line(&format!("{}", arg));
			Ok(arg)
		}
		"io-write" => {
			debug_assert_eq!(args.len(), 1, "`io.write` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stdout.write(&format!("{}", arg));
			Ok(arg)
		}
		"io-write-err" => {
			debug_assert_eq!(args.len(), 1, "`write-err` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stderr.write(&format!("{}", arg));
			Ok(arg)
		}
		"io-read" => {
			// Called as `read ()` — lone arg is `nothing`.
			debug_assert_eq!(args.len(), 1, "`read` arity");
			Ok(match vm.stdin.read_line() {
				Ok(Some(line)) => result_ok(Value::String(Rc::new(line))),
				Ok(None) => result_err(Value::String(Rc::new("EOF".to_string()))),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"io-read-all" => {
			debug_assert_eq!(args.len(), 1, "`read-all` arity");
			Ok(match vm.stdin.read_all() {
				Ok(s) => result_ok(Value::String(Rc::new(s))),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"io-read-file" => {
			let path = expect_string(&args, "read-file");
			Ok(match std::fs::read_to_string(path.as_str()) {
				Ok(contents) => result_ok(Value::String(Rc::new(contents))),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"io-write-file" => {
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
		"io-append-file" => {
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
		"io-file-exists" => {
			let path = expect_string(&args, "file-exists");
			Ok(Value::Bool(std::path::Path::new(path.as_str()).exists()))
		}
		"io-delete-file" => {
			let path = expect_string(&args, "delete-file");
			Ok(match std::fs::remove_file(path.as_str()) {
				Ok(()) => result_ok(Value::Nothing),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"io-args" => {
			// Called as `args ()` — the lone arg is the `nothing` unit.
			debug_assert_eq!(args.len(), 1, "`args` arity");
			// Skip argv[0] (the program path itself).
			let args_list: Vec<Value> = std::env::args()
				.skip(1)
				.map(|a| Value::String(Rc::new(a)))
				.collect();
			Ok(Value::List(Rc::new(args_list)))
		}
		"io-env" => {
			let name = expect_string(&args, "env");
			Ok(option_value(
				std::env::var(name.as_str())
					.ok()
					.map(|v| Value::String(Rc::new(v))),
			))
		}
		"io-exit" => {
			debug_assert_eq!(args.len(), 1, "`exit` arity");
			let code = match &args[0] {
				Value::Int(n) => *n as i32,
				_ => unreachable!("`exit`: expected int"),
			};
			std::process::exit(code);
		}

		"io-read-all-bytes" => {
			// `read-all-bytes ()` — drains stdin without UTF-8 decoding.
			debug_assert_eq!(args.len(), 1, "`read-all-bytes` arity");
			Ok(match vm.stdin.read_all_bytes() {
				Ok(b) => result_ok(Value::Bytes(Rc::new(b))),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"io-read-file-bytes" => {
			let path = expect_string(&args, "read-file-bytes");
			Ok(match std::fs::read(path.as_str()) {
				Ok(b) => result_ok(Value::Bytes(Rc::new(b))),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"io-write-file-bytes" => {
			debug_assert_eq!(args.len(), 2, "`write-file-bytes` arity");
			let (path, contents) = match (&args[0], &args[1]) {
				(Value::String(p), Value::Bytes(c)) => (p, c),
				_ => unreachable!("`write-file-bytes`: expected (string, bytes)"),
			};
			Ok(match std::fs::write(path.as_str(), contents.as_slice()) {
				Ok(()) => result_ok(Value::Nothing),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"io-append-file-bytes" => {
			debug_assert_eq!(args.len(), 2, "`append-file-bytes` arity");
			let (path, contents) = match (&args[0], &args[1]) {
				(Value::String(p), Value::Bytes(c)) => (p, c),
				_ => unreachable!("`append-file-bytes`: expected (string, bytes)"),
			};
			use std::io::Write;
			let result = std::fs::OpenOptions::new()
				.create(true)
				.append(true)
				.open(path.as_str())
				.and_then(|mut f| f.write_all(contents.as_slice()));
			Ok(match result {
				Ok(()) => result_ok(Value::Nothing),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"io-write-bytes" => {
			// Raw byte write to stdout — no newline, no Display formatting.
			debug_assert_eq!(args.len(), 1, "`write-bytes` arity");
			let arg = args.into_iter().next().unwrap();
			match &arg {
				Value::Bytes(b) => vm.stdout.write_bytes(b),
				_ => unreachable!("`write-bytes`: expected bytes"),
			}
			Ok(arg)
		}
		"io-write-err-bytes" => {
			debug_assert_eq!(args.len(), 1, "`write-err-bytes` arity");
			let arg = args.into_iter().next().unwrap();
			match &arg {
				Value::Bytes(b) => vm.stderr.write_bytes(b),
				_ => unreachable!("`write-err-bytes`: expected bytes"),
			}
			Ok(arg)
		}

		"map-empty" => {
			debug_assert_eq!(args.len(), 1, "`map.empty` arity");
			// Called as `empty ()`; the arg is the `nothing` unit.
			Ok(Value::Map(Rc::new(MapData::new())))
		}
		"map-insert" => {
			// args = [hash_dict, m, k, v]
			debug_assert_eq!(args.len(), 4, "`map.insert` arity");
			let mut it = args.into_iter();
			let hash_dict = it.next().unwrap();
			let m_arg = it.next().unwrap();
			let k = it.next().unwrap();
			let v = it.next().unwrap();
			let h = call_hash(vm, &hash_dict, &k)?;
			let m = expect_map_owned(m_arg, "insert");
			Ok(Value::Map(Rc::new(m.inserted(h, k, v))))
		}
		"map-lookup" => {
			// args = [hash_dict, m, k]
			debug_assert_eq!(args.len(), 3, "`map.lookup` arity");
			let mut it = args.into_iter();
			let hash_dict = it.next().unwrap();
			let m_arg = it.next().unwrap();
			let k = it.next().unwrap();
			let h = call_hash(vm, &hash_dict, &k)?;
			let m = expect_map_ref(&m_arg, "lookup");
			Ok(option_value(
				m.find_index(h, &k).map(|i| m.entries[i].1.clone()),
			))
		}
		"map-remove" => {
			debug_assert_eq!(args.len(), 3, "`map.remove` arity");
			let mut it = args.into_iter();
			let hash_dict = it.next().unwrap();
			let m_arg = it.next().unwrap();
			let k = it.next().unwrap();
			let h = call_hash(vm, &hash_dict, &k)?;
			let m = expect_map_owned(m_arg, "remove");
			Ok(Value::Map(Rc::new(m.removed(h, &k))))
		}
		"map-contains-key" => {
			debug_assert_eq!(args.len(), 3, "`map.contains-key` arity");
			let mut it = args.into_iter();
			let hash_dict = it.next().unwrap();
			let m_arg = it.next().unwrap();
			let k = it.next().unwrap();
			let h = call_hash(vm, &hash_dict, &k)?;
			let m = expect_map_ref(&m_arg, "contains-key");
			Ok(Value::Bool(m.find_index(h, &k).is_some()))
		}
		"map-size" => {
			debug_assert_eq!(args.len(), 1, "`map.size` arity");
			let m = expect_map_ref(&args[0], "size");
			Ok(Value::Int(m.entries.len() as i64))
		}
		"map-keys" => {
			debug_assert_eq!(args.len(), 1, "`map.keys` arity");
			let m = expect_map_ref(&args[0], "keys");
			let keys: Vec<Value> = m.entries.iter().map(|(k, _)| k.clone()).collect();
			Ok(Value::List(Rc::new(keys)))
		}
		"map-values" => {
			debug_assert_eq!(args.len(), 1, "`map.values` arity");
			let m = expect_map_ref(&args[0], "values");
			let vs: Vec<Value> = m.entries.iter().map(|(_, v)| v.clone()).collect();
			Ok(Value::List(Rc::new(vs)))
		}
		"map-entries" => {
			debug_assert_eq!(args.len(), 1, "`map.entries` arity");
			let m = expect_map_ref(&args[0], "entries");
			let es: Vec<Value> = m
				.entries
				.iter()
				.map(|(k, v)| Value::Tuple(Rc::new(vec![k.clone(), v.clone()])))
				.collect();
			Ok(Value::List(Rc::new(es)))
		}
		"map-from-entries" => {
			// args = [hash_dict, list]
			debug_assert_eq!(args.len(), 2, "`map.from-entries` arity");
			let mut it = args.into_iter();
			let hash_dict = it.next().unwrap();
			let list_arg = it.next().unwrap();
			let xs = match list_arg {
				Value::List(xs) => xs,
				_ => unreachable!("`from-entries`: expected list"),
			};
			let mut data = MapData::new();
			for entry in xs.iter() {
				let (k, v) = match entry {
					Value::Tuple(t) if t.len() == 2 => (t[0].clone(), t[1].clone()),
					_ => unreachable!("`from-entries`: expected list of 2-tuples"),
				};
				let h = call_hash(vm, &hash_dict, &k)?;
				data = data.inserted(h, k, v);
			}
			Ok(Value::Map(Rc::new(data)))
		}
		"map-merge" => {
			// args = [hash_dict, left, right]; right-wins on conflicts.
			debug_assert_eq!(args.len(), 3, "`map.merge` arity");
			let mut it = args.into_iter();
			let hash_dict = it.next().unwrap();
			let left_arg = it.next().unwrap();
			let right_arg = it.next().unwrap();
			let left = expect_map_ref(&left_arg, "merge").clone();
			let right = expect_map_ref(&right_arg, "merge");
			let mut data = left;
			for (k, v) in right.entries.iter() {
				let h = call_hash(vm, &hash_dict, k)?;
				data = data.inserted(h, k.clone(), v.clone());
			}
			Ok(Value::Map(Rc::new(data)))
		}
		"map-map" => {
			// args = [m, fn]. fn : v -> w (key set is preserved, no rehash).
			debug_assert_eq!(args.len(), 2, "`map.map` arity");
			let mut it = args.into_iter();
			let m_arg = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let m = expect_map_owned(m_arg, "map");
			let mut entries = Vec::with_capacity(m.entries.len());
			for (k, v) in m.entries.iter() {
				let new_v = invoke(vm, fn_arg.clone(), vec![v.clone()])?;
				entries.push((k.clone(), new_v));
			}
			Ok(Value::Map(Rc::new(MapData {
				entries,
				buckets: m.buckets.clone(),
			})))
		}
		"map-filter" => {
			// args = [m, fn]. fn : k -> v -> bool. Predicate-passes keep
			// their slot; rebuild bucket indices over the surviving rows.
			debug_assert_eq!(args.len(), 2, "`map.filter` arity");
			let mut it = args.into_iter();
			let m_arg = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let m = expect_map_owned(m_arg, "filter");
			let mut new_entries: Vec<(Value, Value)> = Vec::new();
			let mut index_map: Vec<Option<usize>> = Vec::with_capacity(m.entries.len());
			for (k, v) in m.entries.iter() {
				let keep = invoke(vm, fn_arg.clone(), vec![k.clone(), v.clone()])?;
				match keep {
					Value::Bool(true) => {
						index_map.push(Some(new_entries.len()));
						new_entries.push((k.clone(), v.clone()));
					}
					Value::Bool(false) => index_map.push(None),
					_ => unreachable!("`map.filter`: predicate must return bool"),
				}
			}
			let mut new_buckets: std::collections::HashMap<i64, Vec<usize>> =
				std::collections::HashMap::new();
			for (h, idxs) in m.buckets.iter() {
				let mapped: Vec<usize> = idxs.iter().filter_map(|&i| index_map[i]).collect();
				if !mapped.is_empty() {
					new_buckets.insert(*h, mapped);
				}
			}
			Ok(Value::Map(Rc::new(MapData {
				entries: new_entries,
				buckets: new_buckets,
			})))
		}
		"ref-new" => {
			debug_assert_eq!(args.len(), 1, "`ref.new` arity");
			let inner = args.into_iter().next().unwrap();
			Ok(Value::Ref(Rc::new(RefCell::new(inner))))
		}
		"ref-get" => {
			debug_assert_eq!(args.len(), 1, "`ref.get` arity");
			let cell = expect_ref(&args[0], "get");
			Ok(cell.borrow().clone())
		}
		"ref-set" => {
			debug_assert_eq!(args.len(), 2, "`ref.set` arity");
			let mut it = args.into_iter();
			let r = it.next().unwrap();
			let v = it.next().unwrap();
			let cell = expect_ref_owned(r, "set");
			*cell.borrow_mut() = v;
			Ok(Value::Nothing)
		}
		"ref-update" => {
			// `update r f` — read once, apply, write back. We release the
			// borrow before calling `f` so user code holding the same ref
			// can read it freely; only the final write reborrows.
			debug_assert_eq!(args.len(), 2, "`ref.update` arity");
			let mut it = args.into_iter();
			let r = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let cell = expect_ref_owned(r, "update");
			let current = cell.borrow().clone();
			let next = invoke(vm, fn_arg, vec![current])?;
			*cell.borrow_mut() = next;
			Ok(Value::Nothing)
		}

		"option-then" => {
			// `option.then o f` — invoke `f` on `some`'s payload; pass
			// `none` through unchanged.
			debug_assert_eq!(args.len(), 2, "`option.then` arity");
			let mut it = args.into_iter();
			let o = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			match o {
				Value::Variant(v) => match v.variant.as_str() {
					"some" => {
						debug_assert_eq!(v.payload.len(), 1, "`some` payload arity");
						invoke(vm, fn_arg, vec![v.payload[0].clone()])
					}
					"none" => Ok(Value::Variant(v)),
					other => unreachable!("`option.then`: unexpected option variant `{}`", other),
				},
				_ => unreachable!("`option.then`: expected option variant"),
			}
		}
		"result-then" => {
			// `result.then r f` — invoke `f` on `ok`'s payload; pass `err`
			// through unchanged.
			debug_assert_eq!(args.len(), 2, "`result.then` arity");
			let mut it = args.into_iter();
			let r = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			match r {
				Value::Variant(v) => match v.variant.as_str() {
					"ok" => {
						debug_assert_eq!(v.payload.len(), 1, "`ok` payload arity");
						invoke(vm, fn_arg, vec![v.payload[0].clone()])
					}
					"err" => Ok(Value::Variant(v)),
					other => unreachable!("`result.then`: unexpected result variant `{}`", other),
				},
				_ => unreachable!("`result.then`: expected result variant"),
			}
		}

		"json-parse" => {
				debug_assert_eq!(args.len(), 1, "`json.parse` arity");
				let s = expect_string(&args, "parse");
				match serde_json::from_str::<serde_json::Value>(s.as_str()) {
					Ok(j) => Ok(result_ok(json_to_pluma(j))),
					Err(e) => Ok(result_err(json_error_record(e.line(), e.column(), &e.to_string()))),
				}
			}
			"json-stringify" => {
				debug_assert_eq!(args.len(), 1, "`json.stringify` arity");
				let j = pluma_to_json(&args[0]);
				Ok(Value::String(Rc::new(
					serde_json::to_string(&j).unwrap_or_default(),
				)))
			}
			"json-stringify-pretty" => {
				debug_assert_eq!(args.len(), 1, "`json.stringify-pretty` arity");
				let j = pluma_to_json(&args[0]);
				Ok(Value::String(Rc::new(
					serde_json::to_string_pretty(&j).unwrap_or_default(),
				)))
			}

			"map-fold" => {
			// args = [m, init, fn]. fn : b -> k -> v -> b.
			debug_assert_eq!(args.len(), 3, "`map.fold` arity");
			let mut it = args.into_iter();
			let m_arg = it.next().unwrap();
			let mut acc = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let m = expect_map_ref(&m_arg, "fold").clone();
			for (k, v) in m.entries.iter() {
				acc = invoke(vm, fn_arg.clone(), vec![acc, k.clone(), v.clone()])?;
			}
			Ok(acc)
		}

		// An unknown tag means a stdlib `.pa` source named a `built-in
		// "..."` that no arm here implements. Codegen doesn't pre-check
		// — `built-in` is internal-only, and a typo is on us.
		other => Err(RuntimeError::new(format!("unknown builtin `{}`", other))),
	}
}

// Pull the hash function (slot 0) out of a hash dict and invoke it on
// `key`, returning the resulting int hash. Used by every Map operation
// that needs to bucket by key.
fn call_hash(vm: &mut VM, dict: &Value, key: &Value) -> Result<i64, RuntimeError> {
	let methods = match dict {
		Value::Dict(d) => d,
		_ => unreachable!("hash dict: expected Dict"),
	};
	let hash_fn = methods
		.get(0)
		.cloned()
		.ok_or_else(|| RuntimeError::new("hash dict: missing slot 0"))?;
	match invoke(vm, hash_fn, vec![key.clone()])? {
		Value::Int(h) => Ok(h),
		_ => unreachable!("hash dict: hash method returned non-int"),
	}
}

fn expect_ref<'a>(v: &'a Value, name: &str) -> &'a Rc<RefCell<Value>> {
	match v {
		Value::Ref(cell) => cell,
		_ => unreachable!("`ref.{}`: expected ref", name),
	}
}

fn expect_ref_owned(v: Value, name: &str) -> Rc<RefCell<Value>> {
	match v {
		Value::Ref(cell) => cell,
		_ => unreachable!("`ref.{}`: expected ref", name),
	}
}

fn expect_map_ref<'a>(v: &'a Value, name: &str) -> &'a MapData {
	match v {
		Value::Map(m) => m,
		_ => unreachable!("`map.{}`: expected map", name),
	}
}

fn expect_map_owned(v: Value, name: &str) -> MapData {
	match v {
		Value::Map(m) => (*m).clone(),
		_ => unreachable!("`map.{}`: expected map", name),
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

fn expect_bytes<'a>(args: &'a [Value], name: &str) -> &'a Rc<Vec<u8>> {
	debug_assert_eq!(args.len(), 1, "`{}` arity", name);
	match &args[0] {
		Value::Bytes(b) => b,
		_ => unreachable!("`{}`: expected bytes", name),
	}
}

fn bytes_contains(haystack: &[u8], needle: &[u8]) -> bool {
	if needle.is_empty() {
		return true;
	}
	if needle.len() > haystack.len() {
		return false;
	}
	haystack.windows(needle.len()).any(|w| w == needle)
}

// Split `b` on every occurrence of `sep`. Mirrors how Rust's
// str::split behaves on non-empty separators, and how `string.split`
// works for the string side. `sep.is_empty()` is handled by the caller.
fn bytes_split(b: &[u8], sep: &[u8]) -> Vec<Vec<u8>> {
	let mut out = Vec::new();
	let mut start = 0;
	let mut i = 0;
	while i + sep.len() <= b.len() {
		if &b[i..i + sep.len()] == sep {
			out.push(b[start..i].to_vec());
			i += sep.len();
			start = i;
		} else {
			i += 1;
		}
	}
	out.push(b[start..].to_vec());
	out
}

fn expect_float(args: &[Value], name: &str) -> f64 {
	debug_assert_eq!(args.len(), 1, "`{}` arity", name);
	match &args[0] {
		Value::Float(n) => *n,
		_ => unreachable!("`{}`: expected float", name),
	}
}

// Build a `core.regex.match` record from a single Captures. The full
// match goes into `text`/`start`/`end`; every named group that fired
// is added to `groups`. Groups that exist in the pattern but didn't
// match (e.g. losing side of an alternation) are simply absent.
fn regex_match_record(re: &regex::Regex, caps: &regex::Captures) -> Value {
	let m = caps.get(0).expect("captures always have group 0");
	let mut groups = MapData::new();
	for name in re.capture_names().flatten() {
		if let Some(g) = caps.name(name) {
			let h = hash_string(name);
			groups = groups.inserted(
				h,
				Value::String(Rc::new(name.to_string())),
				Value::String(Rc::new(g.as_str().to_string())),
			);
		}
	}
	let mut fields = HashMap::with_capacity(4);
	fields.insert(
		"text".to_string(),
		Value::String(Rc::new(m.as_str().to_string())),
	);
	fields.insert("start".to_string(), Value::Int(m.start() as i64));
	fields.insert("end".to_string(), Value::Int(m.end() as i64));
	fields.insert("groups".to_string(), Value::Map(Rc::new(groups)));
	Value::Record(Rc::new(fields))
}

// Hash a string using the same hasher as the `string-hash` builtin —
// keeps json-built maps interoperable with `core.map` lookups.
fn hash_string(s: &str) -> i64 {
	use std::hash::{Hash, Hasher};
	let mut h = std::collections::hash_map::DefaultHasher::new();
	s.hash(&mut h);
	h.finish() as i64
}

// Construct a `core.json.value` variant with the given name and payload.
fn json_variant(variant: &'static str, payload: Vec<Value>) -> Value {
	Value::Variant(Rc::new(VariantData {
		qualified_enum: Rc::new("core.json.value".to_string()),
		variant: Rc::new(variant.to_string()),
		payload,
	}))
}

// Build a `core.json.error` record. Fields match the alias in json.pa.
fn json_error_record(line: usize, col: usize, message: &str) -> Value {
	let mut fields = HashMap::with_capacity(3);
	fields.insert("line".to_string(), Value::Int(line as i64));
	fields.insert("col".to_string(), Value::Int(col as i64));
	fields.insert(
		"message".to_string(),
		Value::String(Rc::new(message.to_string())),
	);
	Value::Record(Rc::new(fields))
}

// Lower a serde_json value into a `core.json.value` Pluma variant. Numbers
// split int vs float by whether they round-trip as i64 — `1.0` parses as
// float, `1` as int (matches the BATTERIES design note's intent of not
// silently downgrading 64-bit ints to f64).
fn json_to_pluma(j: serde_json::Value) -> Value {
	match j {
		serde_json::Value::Null => json_variant("null", vec![]),
		serde_json::Value::Bool(b) => json_variant("bool", vec![Value::Bool(b)]),
		serde_json::Value::Number(n) => {
			if let Some(i) = n.as_i64() {
				json_variant("int", vec![Value::Int(i)])
			} else if let Some(f) = n.as_f64() {
				json_variant("float", vec![Value::Float(f)])
			} else {
				// u64 too large for i64 — fall back to float (lossy, but
				// the alternative is panicking on a valid JSON document).
				json_variant("float", vec![Value::Float(n.as_f64().unwrap_or(0.0))])
			}
		}
		serde_json::Value::String(s) => json_variant("string", vec![Value::String(Rc::new(s))]),
		serde_json::Value::Array(xs) => {
			let items: Vec<Value> = xs.into_iter().map(json_to_pluma).collect();
			json_variant("array", vec![Value::List(Rc::new(items))])
		}
		serde_json::Value::Object(obj) => {
			let mut data = MapData::new();
			// `preserve_order` keeps these in source order.
			for (k, v) in obj.into_iter() {
				let h = hash_string(&k);
				let key = Value::String(Rc::new(k));
				data = data.inserted(h, key, json_to_pluma(v));
			}
			json_variant("object", vec![Value::Map(Rc::new(data))])
		}
	}
}

// Lift a `core.json.value` back into serde_json. The arity-check
// `unreachable!`s catch a malformed value built by something other than
// the parser; the type system guarantees the variant name and payload
// shape match.
fn pluma_to_json(v: &Value) -> serde_json::Value {
	let var = match v {
		Value::Variant(var) => var,
		_ => unreachable!("`json.stringify`: expected json.value variant"),
	};
	match var.variant.as_str() {
		"null" => serde_json::Value::Null,
		"bool" => match &var.payload[..] {
			[Value::Bool(b)] => serde_json::Value::Bool(*b),
			_ => unreachable!("`json.value.bool`: expected single bool payload"),
		},
		"int" => match &var.payload[..] {
			[Value::Int(n)] => serde_json::Value::Number((*n).into()),
			_ => unreachable!("`json.value.int`: expected single int payload"),
		},
		"float" => match &var.payload[..] {
			[Value::Float(f)] => serde_json::Number::from_f64(*f)
				.map(serde_json::Value::Number)
				.unwrap_or(serde_json::Value::Null),
			_ => unreachable!("`json.value.float`: expected single float payload"),
		},
		"string" => match &var.payload[..] {
			[Value::String(s)] => serde_json::Value::String((**s).clone()),
			_ => unreachable!("`json.value.string`: expected single string payload"),
		},
		"array" => match &var.payload[..] {
			[Value::List(xs)] => {
				serde_json::Value::Array(xs.iter().map(pluma_to_json).collect())
			}
			_ => unreachable!("`json.value.array`: expected single list payload"),
		},
		"object" => match &var.payload[..] {
			[Value::Map(m)] => {
				let mut obj = serde_json::Map::new();
				for (k, v) in m.entries.iter() {
					let key = match k {
						Value::String(s) => (**s).clone(),
						_ => unreachable!("`json.value.object`: keys must be strings"),
					};
					obj.insert(key, pluma_to_json(v));
				}
				serde_json::Value::Object(obj)
			}
			_ => unreachable!("`json.value.object`: expected single map payload"),
		},
		other => unreachable!("`json.stringify`: unexpected variant `{}`", other),
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
		Value::Builtin(b) => call_builtin(vm, b.as_ref(), args),
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
