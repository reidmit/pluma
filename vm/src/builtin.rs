// Builtin dispatch. Each `Value::Builtin(tag)` lands here when invoked —
// `call_builtin` matches the tag against this file's arms and runs the
// corresponding Rust implementation. Operator handlers (arithmetic,
// comparison, etc.) are inlined into the VM dispatch loop instead; this
// file is only the named-builtin path plus the cross-call `invoke` helper.

use crate::value::{DictData, TaskRepr, Value, VariantData};
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
			Ok(Value::Nothing)
		}
		"debug" => {
			debug_assert_eq!(args.len(), 1, "`debug` arity");
			let arg = args.into_iter().next().unwrap();
			let (module, line) = vm.current_call_site();
			vm.stdout
				.write_line(&format!("[{}:{}] {}", module, line, arg));
			Ok(arg)
		}
		"to-string" => {
			debug_assert_eq!(args.len(), 1, "`to-string` arity");
			Ok(Value::String(Rc::new(format!("{}", args[0]))))
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
		// The primitive `hash` instances all delegate to one shared function so
		// the `wire` codec can rebuild dict buckets identically (see
		// `value::primitive_hash`).
		"int-hash" | "float-hash" | "string-hash" | "bool-hash" => Ok(Value::Int(
			crate::value::primitive_hash(&args[0])
				.unwrap_or_else(|| unreachable!("`{}` expects a primitive", tag)),
		)),
		"list-length" => {
			let xs = expect_list(&args, "length");
			Ok(Value::Int(xs.len() as i64))
		}
		"list-get" => {
			let xs = match &args[0] {
				Value::List(xs) => xs.borrow(),
				_ => unreachable!("`get`: expected list"),
			};
			let i = match &args[1] {
				Value::Int(n) => *n,
				_ => unreachable!("`get`: expected int"),
			};
			if i < 0 || (i as usize) >= xs.len() {
				Err(RuntimeError::new(format!(
					"list.get: index {i} out of bounds (length {})",
					xs.len()
				)))
			} else {
				Ok(xs[i as usize].clone())
			}
		}
		"list-build" => {
			// Tabulate: `[f 0, f 1, ..., f (n-1)]` in one pass.
			debug_assert_eq!(args.len(), 2, "`build` arity");
			let mut it = args.into_iter();
			let n = match it.next().unwrap() {
				Value::Int(n) => n.max(0) as usize,
				_ => unreachable!("`build`: expected int"),
			};
			let f = it.next().unwrap();
			let mut out = Vec::with_capacity(n);
			for i in 0..n {
				out.push(invoke(vm, f.clone(), vec![Value::Int(i as i64)])?);
			}
			Ok(Value::list(out))
		}
		"list-collect" => {
			// Like `build`, but `f` returns an `option`; keep the `some`s in
			// index order, dropping `none`s (so the result may be shorter).
			debug_assert_eq!(args.len(), 2, "`collect` arity");
			let mut it = args.into_iter();
			let n = match it.next().unwrap() {
				Value::Int(n) => n.max(0) as usize,
				_ => unreachable!("`collect`: expected int"),
			};
			let f = it.next().unwrap();
			let mut out = Vec::new();
			for i in 0..n {
				match invoke(vm, f.clone(), vec![Value::Int(i as i64)])? {
					Value::Variant(v) => match v.variant.as_str() {
						"some" => out.push(v.payload[0].clone()),
						"none" => {}
						other => unreachable!("`collect`: expected option, got `{}`", other),
					},
					_ => unreachable!("`collect`: expected option variant"),
				}
			}
			Ok(Value::list(out))
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
		"math-log" => Ok(Value::Float(expect_float(&args, "log").ln())),
		"math-log10" => Ok(Value::Float(expect_float(&args, "log10").log10())),
		"math-log2" => Ok(Value::Float(expect_float(&args, "log2").log2())),
		"math-exp" => Ok(Value::Float(expect_float(&args, "exp").exp())),
		"math-sin" => Ok(Value::Float(expect_float(&args, "sin").sin())),
		"math-cos" => Ok(Value::Float(expect_float(&args, "cos").cos())),
		"string-to-bytes" => {
			let s = expect_string(&args, "to-bytes");
			Ok(Value::Bytes(Rc::new(s.as_bytes().to_vec())))
		}
		// `wire` codec. Both take the reified schema
		// (a `wire-schema` value built by codegen from the static type) as the
		// hidden first arg — the `wire a` dictionary — followed by the
		// value/bytes. `encode` can't fail (the type checker guarantees the
		// value matches the schema); `decode` returns `result a wire-error`.
		"wire-encode" => {
			debug_assert_eq!(args.len(), 2, "`wire-encode` arity");
			let schema = crate::wire::schema_from_value(&args[0])
				.unwrap_or_else(|| unreachable!("`wire-encode`: malformed schema"));
			let mut out = Vec::new();
			crate::wire::encode(&schema, &args[1], &mut out);
			Ok(Value::Bytes(Rc::new(out)))
		}
		"wire-decode" => {
			debug_assert_eq!(args.len(), 2, "`wire-decode` arity");
			let schema = crate::wire::schema_from_value(&args[0])
				.unwrap_or_else(|| unreachable!("`wire-decode`: malformed schema"));
			let bytes = match &args[1] {
				Value::Bytes(b) => b,
				_ => unreachable!("`wire-decode`: expected bytes"),
			};
			match crate::wire::decode_all(&schema, bytes) {
				Ok(v) => Ok(result_ok(v)),
				Err(e) => Ok(result_err(e.to_value())),
			}
		}
		// The structural fingerprint of the value's TYPE, for version-skew
		// detection. The value (arg 1) is ignored — only the schema dict (arg 0)
		// matters; it's there because `fingerprint :: fun a -> int` dispatches
		// on `a`, so the schema arrives the same way as for encode/decode.
		"wire-fingerprint" => {
			debug_assert_eq!(args.len(), 2, "`wire-fingerprint` arity");
			let schema = crate::wire::schema_from_value(&args[0])
				.unwrap_or_else(|| unreachable!("`wire-fingerprint`: malformed schema"));
			Ok(Value::Int(crate::wire::fingerprint(&schema)))
		}
		"bytes-length" => {
			let b = expect_bytes(&args, "length");
			Ok(Value::Int(b.len() as i64))
		}
		"bytes-get" => {
			// O(1) unchecked byte access (the primitive the byte ops build on).
			let b = match &args[0] {
				Value::Bytes(b) => b,
				_ => unreachable!("`bytes.get`: expected bytes"),
			};
			let i = match &args[1] {
				Value::Int(n) => *n,
				_ => unreachable!("`bytes.get`: expected int"),
			};
			if i < 0 || (i as usize) >= b.len() {
				Err(RuntimeError::new(format!(
					"bytes.get: index {i} out of bounds (length {})",
					b.len()
				)))
			} else {
				Ok(Value::Int(b[i as usize] as i64))
			}
		}
		"bytes-build" => {
			// Tabulate a byte sequence; the builder's int result is taken mod 256.
			debug_assert_eq!(args.len(), 2, "`bytes.build` arity");
			let mut it = args.into_iter();
			let n = match it.next().unwrap() {
				Value::Int(n) => n.max(0) as usize,
				_ => unreachable!("`bytes.build`: expected int"),
			};
			let f = it.next().unwrap();
			let mut out = Vec::with_capacity(n);
			for i in 0..n {
				match invoke(vm, f.clone(), vec![Value::Int(i as i64)])? {
					Value::Int(v) => out.push((v & 0xff) as u8),
					_ => unreachable!("`bytes.build`: builder must return int"),
				}
			}
			Ok(Value::Bytes(Rc::new(out)))
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
		"bytes-as-string" => {
			// Unchecked reinterpret of bytes as a string. `bytes.to-string`
			// validates UTF-8 in Pluma first, so the bytes are well-formed here;
			// `from_utf8` keeps it memory-safe regardless.
			let b = expect_bytes(&args, "as-string");
			match std::str::from_utf8(b) {
				Ok(s) => Ok(Value::String(Rc::new(s.to_string()))),
				Err(_) => Ok(Value::String(Rc::new(
					String::from_utf8_lossy(b).into_owned(),
				))),
			}
		}
		"bytes-compare" => match (&args[0], &args[1]) {
			(Value::Bytes(a), Value::Bytes(b)) => Ok(ordering_variant(a.as_slice().cmp(b.as_slice()))),
			_ => unreachable!("`bytes-compare` expects (bytes, bytes)"),
		},
		"bytes-hash" => Ok(Value::Int(
			crate::value::primitive_hash(&args[0])
				.unwrap_or_else(|| unreachable!("`bytes-hash` expects bytes")),
		)),
		"io-print" => {
			debug_assert_eq!(args.len(), 1, "`io.print` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stdout.write_line(&format!("{}", arg));
			Ok(Value::Nothing)
		}
		"io-print-err" => {
			debug_assert_eq!(args.len(), 1, "`print-err` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stderr.write_line(&format!("{}", arg));
			Ok(Value::Nothing)
		}
		"io-write" => {
			debug_assert_eq!(args.len(), 1, "`io.write` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stdout.write(&format!("{}", arg));
			Ok(Value::Nothing)
		}
		"io-write-err" => {
			debug_assert_eq!(args.len(), 1, "`write-err` arity");
			let arg = args.into_iter().next().unwrap();
			vm.stderr.write(&format!("{}", arg));
			Ok(Value::Nothing)
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
		"io-is-dir" => {
			let path = expect_string(&args, "is-dir");
			Ok(Value::Bool(std::path::Path::new(path.as_str()).is_dir()))
		}
		"io-make-dir" => {
			// Creates the directory and any missing parents (mkdir -p).
			// Succeeds silently if it already exists.
			let path = expect_string(&args, "make-dir");
			Ok(match std::fs::create_dir_all(path.as_str()) {
				Ok(()) => result_ok(Value::Nothing),
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"io-read-dir" => {
			// Entry names only (not full paths), sorted for deterministic
			// builds. Hidden dotfiles are included.
			let path = expect_string(&args, "read-dir");
			Ok(match std::fs::read_dir(path.as_str()) {
				Ok(entries) => {
					let mut names: Vec<String> = Vec::new();
					let mut read_err: Option<String> = None;
					for entry in entries {
						match entry {
							Ok(e) => names.push(e.file_name().to_string_lossy().into_owned()),
							Err(e) => {
								read_err = Some(e.to_string());
								break;
							}
						}
					}
					match read_err {
						Some(msg) => result_err(Value::String(Rc::new(msg))),
						None => {
							names.sort();
							let list: Vec<Value> = names
								.into_iter()
								.map(|n| Value::String(Rc::new(n)))
								.collect();
							result_ok(Value::list(list))
						}
					}
				}
				Err(e) => result_err(Value::String(Rc::new(e.to_string()))),
			})
		}
		"io-args" => {
			// Called as `args ()` — the lone arg is the `nothing` unit.
			debug_assert_eq!(args.len(), 1, "`args` arity");
			// `vm.args` is already stripped of the interpreter and script
			// path by the CLI, so this is exactly the program's own arguments.
			let args_list: Vec<Value> = vm
				.args
				.iter()
				.map(|a| Value::String(Rc::new(a.clone())))
				.collect();
			Ok(Value::list(args_list))
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

		"io-fail" => {
			// `io.fail msg` — stop the program with `msg` on stderr and a
			// nonzero exit. A program-controlled abort, so it surfaces as a
			// user-abort RuntimeError rather than a VM fault.
			debug_assert_eq!(args.len(), 1, "`fail` arity");
			let msg = expect_string(&args, "fail");
			Err(RuntimeError::user_abort(msg.to_string()))
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
			Ok(Value::Nothing)
		}
		"io-write-err-bytes" => {
			debug_assert_eq!(args.len(), 1, "`write-err-bytes` arity");
			let arg = args.into_iter().next().unwrap();
			match &arg {
				Value::Bytes(b) => vm.stderr.write_bytes(b),
				_ => unreachable!("`write-err-bytes`: expected bytes"),
			}
			Ok(Value::Nothing)
		}

		"dict-empty" => {
			debug_assert_eq!(args.len(), 1, "`dict.empty` arity");
			// Called as `empty ()`; the arg is the `nothing` unit.
			Ok(Value::Dict(Rc::new(DictData::new())))
		}
		"dict-insert" => {
			// args = [hash_dict, m, k, v]
			debug_assert_eq!(args.len(), 4, "`dict.insert` arity");
			let mut it = args.into_iter();
			let hash_dict = it.next().unwrap();
			let m_arg = it.next().unwrap();
			let k = it.next().unwrap();
			let v = it.next().unwrap();
			let h = call_hash(vm, &hash_dict, &k)?;
			let m = expect_dict_owned(m_arg, "insert");
			Ok(Value::Dict(Rc::new(m.inserted(h, k, v))))
		}
		"dict-lookup" => {
			// args = [hash_dict, m, k]
			debug_assert_eq!(args.len(), 3, "`dict.lookup` arity");
			let mut it = args.into_iter();
			let hash_dict = it.next().unwrap();
			let m_arg = it.next().unwrap();
			let k = it.next().unwrap();
			let h = call_hash(vm, &hash_dict, &k)?;
			let m = expect_dict_ref(&m_arg, "lookup");
			Ok(option_value(
				m.find_index(h, &k).map(|i| m.entries[i].1.clone()),
			))
		}
		"dict-remove" => {
			debug_assert_eq!(args.len(), 3, "`dict.remove` arity");
			let mut it = args.into_iter();
			let hash_dict = it.next().unwrap();
			let m_arg = it.next().unwrap();
			let k = it.next().unwrap();
			let h = call_hash(vm, &hash_dict, &k)?;
			let m = expect_dict_owned(m_arg, "remove");
			Ok(Value::Dict(Rc::new(m.removed(h, &k))))
		}
		"dict-size" => {
			debug_assert_eq!(args.len(), 1, "`dict.size` arity");
			let m = expect_dict_ref(&args[0], "size");
			Ok(Value::Int(m.entries.len() as i64))
		}
		"dict-entries" => {
			debug_assert_eq!(args.len(), 1, "`dict.entries` arity");
			let m = expect_dict_ref(&args[0], "entries");
			let es: Vec<Value> = m
				.entries
				.iter()
				.map(|(k, v)| Value::Tuple(Rc::new(vec![k.clone(), v.clone()])))
				.collect();
			Ok(Value::list(es))
		}
		"dict-map" => {
			// args = [m, fn]. fn : v -> w (key set is preserved, no rehash).
			debug_assert_eq!(args.len(), 2, "`dict.map` arity");
			let mut it = args.into_iter();
			let m_arg = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let m = expect_dict_owned(m_arg, "map");
			let mut entries = Vec::with_capacity(m.entries.len());
			for (k, v) in m.entries.iter() {
				let new_v = invoke(vm, fn_arg.clone(), vec![v.clone()])?;
				entries.push((k.clone(), new_v));
			}
			Ok(Value::Dict(Rc::new(DictData {
				entries,
				buckets: m.buckets.clone(),
			})))
		}
		"dict-filter" => {
			// args = [m, fn]. fn : k -> v -> bool. Predicate-passes keep
			// their slot; rebuild bucket indices over the surviving rows.
			debug_assert_eq!(args.len(), 2, "`dict.filter` arity");
			let mut it = args.into_iter();
			let m_arg = it.next().unwrap();
			let fn_arg = it.next().unwrap();
			let m = expect_dict_owned(m_arg, "filter");
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
					_ => unreachable!("`dict.filter`: predicate must return bool"),
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
			Ok(Value::Dict(Rc::new(DictData {
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

		// The one in-place mutation on a list: overwrite slot `i` with `v`,
		// returning `nothing`. Partial like `list.get` (OOB stops the
		// program). The list's backing is shared, so the write is visible
		// through every alias — the deliberate escape hatch from list
		// immutability that `list.sort` and the linear builders ride on.
		"list-set" => {
			debug_assert_eq!(args.len(), 3, "`list.set` arity");
			let cells = match &args[0] {
				Value::List(cells) => cells,
				_ => unreachable!("`list.set`: expected list"),
			};
			let i = match &args[1] {
				Value::Int(n) => *n,
				_ => unreachable!("`list.set`: expected int"),
			};
			let mut cells = cells.borrow_mut();
			if i < 0 || (i as usize) >= cells.len() {
				return Err(RuntimeError::new(format!(
					"list.set: index {i} out of bounds (length {})",
					cells.len()
				)));
			}
			cells[i as usize] = args[2].clone();
			Ok(Value::Nothing)
		}

		// Assertions return a `result nothing string`: `ok ()` to pass,
		// `err message` to fail. They never abort — a case body's final
		// result is what `pluma test` reads. (The `is-*` predicates live
		// in `assert.pa` as pure Pluma; only the value-formatting checks
		// and the `all` combinator need Rust.)
		"uuid-v4" => {
			debug_assert_eq!(args.len(), 1, "`uuid.v4` arity");
			Ok(Value::String(Rc::new(uuid::Uuid::new_v4().to_string())))
		}
		"uuid-v7" => {
			debug_assert_eq!(args.len(), 1, "`uuid.v7` arity");
			Ok(Value::String(Rc::new(uuid::Uuid::now_v7().to_string())))
		}
		"uuid-parse" => {
			let s = expect_string(&args, "uuid.parse");
			match uuid::Uuid::try_parse(s) {
				Ok(u) => Ok(result_ok(Value::String(Rc::new(u.to_string())))),
				Err(e) => Ok(result_err(Value::String(Rc::new(e.to_string())))),
			}
		}
		"random-int" => {
			use rand::RngExt as _;
			debug_assert_eq!(args.len(), 1, "`random.int` arity");
			Ok(Value::Int(rand::rng().random_range(0..i64::MAX)))
		}
		"random-float" => {
			use rand::RngExt as _;
			debug_assert_eq!(args.len(), 1, "`random.float` arity");
			Ok(Value::Float(rand::rng().random::<f64>()))
		}
		"random-bytes" => {
			use rand::Rng as _;
			debug_assert_eq!(args.len(), 1, "`random.bytes` arity");
			match &args[0] {
				Value::Int(n) if *n < 0 => Ok(result_err(Value::String(Rc::new(format!(
					"random.bytes: negative length: {}",
					n
				))))),
				Value::Int(n) => {
					let mut buf = vec![0u8; *n as usize];
					rand::rng().fill_bytes(&mut buf);
					Ok(result_ok(Value::Bytes(Rc::new(buf))))
				}
				_ => unreachable!("`random.bytes`: expected int"),
			}
		}
		"random-int-range" => {
			use rand::RngExt as _;
			debug_assert_eq!(args.len(), 2, "`random.int-range` arity");
			match (&args[0], &args[1]) {
				(Value::Int(lo), Value::Int(hi)) if *lo >= *hi => Ok(result_err(Value::String(Rc::new(
					format!("random.int-range: low ({}) >= high ({})", lo, hi),
				)))),
				(Value::Int(lo), Value::Int(hi)) => {
					Ok(result_ok(Value::Int(rand::rng().random_range(*lo..*hi))))
				}
				_ => unreachable!("`random.int-range`: expected (int, int)"),
			}
		}
		// ---- core.time ----------------------------------------------------
		// Instants are i64 nanoseconds since the Unix epoch (UTC); durations
		// are i64 nanosecond spans. The `.pa` layer builds every higher-level
		// operation (unit constructors, arithmetic, comparisons) out of the
		// box/unbox builtins below, so only genuinely-native work (the clock,
		// calendar breakdown, formatting, parsing) lands here.
		// --- core.task primitives. Each builds a cold `Value::Task` recipe; the
		// driver in `vm::task` runs them. They never perform I/O or suspend
		// here — that's the driver's job. ---
		"task-return" => Ok(Value::Task(Rc::new(TaskRepr::Pure(
			args.into_iter().next().unwrap_or(Value::Nothing),
		)))),
		"task-fail" => Ok(Value::Task(Rc::new(TaskRepr::Fail(
			args.into_iter().next().unwrap_or(Value::Nothing),
		)))),
		"task-yield" => Ok(Value::Task(Rc::new(TaskRepr::Yield))),
		"task-sleep" => {
			let nanos = expect_duration(&args, "sleep");
			Ok(Value::Task(Rc::new(TaskRepr::Sleep(nanos))))
		}
		"task-then" => {
			let mut it = args.into_iter();
			let task = Box::new(it.next().unwrap_or(Value::Nothing));
			let k = it.next().unwrap_or(Value::Nothing);
			Ok(Value::Task(Rc::new(TaskRepr::Then { task, k })))
		}
		"task-or-else" => {
			let mut it = args.into_iter();
			let task = Box::new(it.next().unwrap_or(Value::Nothing));
			let recover = it.next().unwrap_or(Value::Nothing);
			Ok(Value::Task(Rc::new(TaskRepr::OrElse { task, recover })))
		}
		"task-attempt" => {
			let task = Box::new(args.into_iter().next().unwrap_or(Value::Nothing));
			Ok(Value::Task(Rc::new(TaskRepr::Attempt { task })))
		}
		"task-map" => {
			// `task.map t f` — task-first, so `t | task.map f` works.
			let mut it = args.into_iter();
			let task = Box::new(it.next().unwrap_or(Value::Nothing));
			let f = it.next().unwrap_or(Value::Nothing);
			Ok(Value::Task(Rc::new(TaskRepr::Map { task, f })))
		}
		"task-shielded" => {
			let task = Box::new(args.into_iter().next().unwrap_or(Value::Nothing));
			Ok(Value::Task(Rc::new(TaskRepr::Shielded { task })))
		}
		// --- structured-concurrency kernel (see vm::task) ---
		"scope-new" => {
			// `scope-new is-manual body` — what the `scope` keyword lowers to.
			let mut it = args.into_iter();
			let manual = matches!(it.next(), Some(Value::Bool(true)));
			let body_fn = it.next().unwrap_or(Value::Nothing);
			Ok(Value::Task(Rc::new(TaskRepr::Scope { manual, body_fn })))
		}
		"scope-spawn" => {
			let mut it = args.into_iter();
			let sid = match it.next() {
				Some(Value::ScopeHandle(s)) => s,
				_ => return Err(RuntimeError::new("scope-spawn: expected a scope handle")),
			};
			let task = it.next().unwrap_or(Value::Nothing);
			let fid = vm.sched_spawn(sid, task);
			Ok(Value::Task(Rc::new(TaskRepr::Handle(fid))))
		}
		"scope-cancel" => {
			if let Some(Value::ScopeHandle(sid)) = args.into_iter().next() {
				vm.sched_cancel(sid);
			}
			Ok(Value::Nothing)
		}
		"scope-cancel-after" => {
			let sid = match args.first() {
				Some(Value::ScopeHandle(s)) => *s,
				_ => {
					return Err(RuntimeError::new(
						"scope-cancel-after: expected a scope handle",
					));
				}
			};
			let ns = match args.get(1) {
				Some(Value::Duration(n)) => *n,
				_ => return Err(RuntimeError::new("scope-cancel-after: expected a duration")),
			};
			vm.sched_cancel_after(sid, ns);
			Ok(Value::Nothing)
		}
		"scope-next" => match args.into_iter().next() {
			Some(Value::ScopeHandle(sid)) => Ok(Value::Task(Rc::new(TaskRepr::Next(sid)))),
			_ => Err(RuntimeError::new("scope-next: expected a scope handle")),
		},
		"time-now" => {
			debug_assert_eq!(args.len(), 1, "`now` arity");
			Ok(Value::Instant(jiff::Timestamp::now().as_nanosecond() as i64))
		}
		"time-monotonic" => {
			debug_assert_eq!(args.len(), 1, "`monotonic` arity");
			use std::sync::OnceLock;
			static START: OnceLock<std::time::Instant> = OnceLock::new();
			let start = START.get_or_init(std::time::Instant::now);
			Ok(Value::Duration(start.elapsed().as_nanos() as i64))
		}
		"time-sleep" => {
			let nanos = expect_duration(&args, "sleep");
			if nanos > 0 {
				std::thread::sleep(std::time::Duration::from_nanos(nanos as u64));
			}
			Ok(Value::Nothing)
		}
		"time-to-unix-nanos" => {
			let nanos = expect_instant(&args, "to-unix-nanos");
			Ok(Value::Int(nanos))
		}
		"time-from-unix-nanos" => {
			let nanos = expect_int(&args, "from-unix-nanos");
			Ok(Value::Instant(nanos))
		}
		"time-duration-as-nanos" => {
			let nanos = expect_duration(&args, "as-nanos");
			Ok(Value::Int(nanos))
		}
		"time-duration-of-nanos" => {
			let nanos = expect_int(&args, "nanos");
			Ok(Value::Duration(nanos))
		}
		"time-parts" => {
			let nanos = expect_instant(&args, "parts");
			Ok(time_parts_record(nanos))
		}
		"time-make" => {
			debug_assert_eq!(args.len(), 7, "`make` arity");
			let ints: Vec<i64> = args
				.iter()
				.map(|a| match a {
					Value::Int(n) => *n,
					_ => unreachable!("`time.make`: expected ints"),
				})
				.collect();
			Ok(
				match make_instant(
					ints[0], ints[1], ints[2], ints[3], ints[4], ints[5], ints[6],
				) {
					Ok(nanos) => result_ok(Value::Instant(nanos)),
					Err(e) => result_err(Value::String(Rc::new(e))),
				},
			)
		}
		"time-format" => {
			debug_assert_eq!(args.len(), 2, "`format` arity");
			let (nanos, fmt) = match (&args[0], &args[1]) {
				(Value::Instant(n), Value::String(s)) => (*n, s),
				_ => unreachable!("`format`: expected (instant, string)"),
			};
			Ok(Value::String(Rc::new(time_format(nanos, fmt))))
		}
		"time-parse-iso" => {
			let s = expect_string(&args, "parse-iso");
			Ok(match parse_iso(s.as_str()) {
				Ok(nanos) => result_ok(Value::Instant(nanos)),
				Err(e) => result_err(Value::String(Rc::new(e))),
			})
		}
		"time-parse" => {
			debug_assert_eq!(args.len(), 2, "`parse` arity");
			let (fmt, input) = match (&args[0], &args[1]) {
				(Value::String(f), Value::String(i)) => (f, i),
				_ => unreachable!("`parse`: expected (string, string)"),
			};
			Ok(match parse_with_format(fmt, input) {
				Ok(nanos) => result_ok(Value::Instant(nanos)),
				Err(e) => result_err(Value::String(Rc::new(e))),
			})
		}
		"time-add-months" => {
			debug_assert_eq!(args.len(), 2, "`add-months` arity");
			let (nanos, n) = match (&args[0], &args[1]) {
				(Value::Instant(t), Value::Int(n)) => (*t, *n),
				_ => unreachable!("`add-months`: expected (instant, int)"),
			};
			Ok(match shift_calendar(nanos, n, 0) {
				Ok(nanos) => result_ok(Value::Instant(nanos)),
				Err(e) => result_err(Value::String(Rc::new(e))),
			})
		}
		"time-add-years" => {
			debug_assert_eq!(args.len(), 2, "`add-years` arity");
			let (nanos, n) = match (&args[0], &args[1]) {
				(Value::Instant(t), Value::Int(n)) => (*t, *n),
				_ => unreachable!("`add-years`: expected (instant, int)"),
			};
			Ok(match shift_calendar(nanos, 0, n) {
				Ok(nanos) => result_ok(Value::Instant(nanos)),
				Err(e) => result_err(Value::String(Rc::new(e))),
			})
		}

		// An unknown tag means a stdlib `.pa` source named a `built-in
		// "..."` that no arm here implements. Codegen doesn't pre-check
		// — `built-in` is internal-only, and a typo is on us.
		other => Err(RuntimeError::new(format!("unknown builtin `{}`", other))),
	}
}

// Pull the hash function (slot 0) out of a hash dict and invoke it on
// `key`, returning the resulting int hash. Used by every `core.dict`
// operation that needs to bucket by key.
fn call_hash(vm: &mut VM, dict: &Value, key: &Value) -> Result<i64, RuntimeError> {
	let methods = match dict {
		Value::MethodDict(d) => d,
		_ => unreachable!("hash dict: expected method dict"),
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

fn expect_dict_ref<'a>(v: &'a Value, name: &str) -> &'a DictData {
	match v {
		Value::Dict(m) => m,
		_ => unreachable!("`dict.{}`: expected dict", name),
	}
}

fn expect_dict_owned(v: Value, name: &str) -> DictData {
	match v {
		Value::Dict(m) => (*m).clone(),
		_ => unreachable!("`dict.{}`: expected dict", name),
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

fn expect_list<'a>(args: &'a [Value], name: &str) -> std::cell::Ref<'a, Vec<Value>> {
	debug_assert_eq!(args.len(), 1, "`{}` arity", name);
	match &args[0] {
		Value::List(xs) => xs.borrow(),
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

fn expect_int(args: &[Value], name: &str) -> i64 {
	debug_assert_eq!(args.len(), 1, "`{}` arity", name);
	match &args[0] {
		Value::Int(n) => *n,
		_ => unreachable!("`{}`: expected int", name),
	}
}

fn expect_instant(args: &[Value], name: &str) -> i64 {
	debug_assert_eq!(args.len(), 1, "`{}` arity", name);
	match &args[0] {
		Value::Instant(n) => *n,
		_ => unreachable!("`{}`: expected instant", name),
	}
}

fn expect_duration(args: &[Value], name: &str) -> i64 {
	debug_assert_eq!(args.len(), 1, "`{}` arity", name);
	match &args[0] {
		Value::Duration(n) => *n,
		_ => unreachable!("`{}`: expected duration", name),
	}
}

// ---- core.time helpers --------------------------------------------------

// Lower a nanosecond instant to a UTC civil datetime, or `None` if it falls
// outside jiff's representable range (only reachable via deliberately
// extreme `from-unix-nanos` input).
fn instant_to_utc(nanos: i64) -> Option<jiff::civil::DateTime> {
	let ts = jiff::Timestamp::from_nanosecond(nanos as i128).ok()?;
	Some(jiff::tz::Offset::UTC.to_datetime(ts))
}

// Break an instant into a UTC calendar `parts` record. weekday is 1=Monday
// .. 7=Sunday (ISO 8601). An out-of-range instant degrades to the epoch
// rather than crashing.
fn time_parts_record(nanos: i64) -> Value {
	let dt = instant_to_utc(nanos).unwrap_or_else(|| jiff::civil::DateTime::default());
	let mut fields = HashMap::with_capacity(8);
	fields.insert("year".to_string(), Value::Int(dt.year() as i64));
	fields.insert("month".to_string(), Value::Int(dt.month() as i64));
	fields.insert("day".to_string(), Value::Int(dt.day() as i64));
	fields.insert("hour".to_string(), Value::Int(dt.hour() as i64));
	fields.insert("minute".to_string(), Value::Int(dt.minute() as i64));
	fields.insert("second".to_string(), Value::Int(dt.second() as i64));
	fields.insert(
		"nanosecond".to_string(),
		Value::Int(dt.subsec_nanosecond() as i64),
	);
	fields.insert(
		"weekday".to_string(),
		Value::Int(dt.weekday().to_monday_one_offset() as i64),
	);
	Value::Record(Rc::new(fields))
}

// Build a UTC instant from calendar components, validating each (jiff
// rejects e.g. month 13 or Feb 30). Returns nanoseconds since the epoch.
fn make_instant(
	year: i64,
	month: i64,
	day: i64,
	hour: i64,
	minute: i64,
	second: i64,
	nanosecond: i64,
) -> Result<i64, String> {
	// Narrow each field to the width jiff expects, erroring (rather than
	// silently wrapping, as `as` would) on anything out of range. jiff's
	// `DateTime::new` then rejects combinations that pass the width check but
	// aren't real dates (month 13, Feb 30, hour 24, ...).
	let year: i16 = year
		.try_into()
		.map_err(|_| format!("time: year out of range: {}", year))?;
	let month: i8 = month
		.try_into()
		.map_err(|_| format!("time: month out of range: {}", month))?;
	let day: i8 = day
		.try_into()
		.map_err(|_| format!("time: day out of range: {}", day))?;
	let hour: i8 = hour
		.try_into()
		.map_err(|_| format!("time: hour out of range: {}", hour))?;
	let minute: i8 = minute
		.try_into()
		.map_err(|_| format!("time: minute out of range: {}", minute))?;
	let second: i8 = second
		.try_into()
		.map_err(|_| format!("time: second out of range: {}", second))?;
	let nanosecond: i32 = nanosecond
		.try_into()
		.map_err(|_| format!("time: nanosecond out of range: {}", nanosecond))?;
	let dt = jiff::civil::DateTime::new(year, month, day, hour, minute, second, nanosecond)
		.map_err(|e| format!("time: invalid date/time: {}", e))?;
	let ts = dt
		.to_zoned(jiff::tz::TimeZone::UTC)
		.map_err(|e| format!("time: {}", e))?
		.timestamp();
	Ok(ts.as_nanosecond() as i64)
}

// strftime-format an instant in UTC. A bad format specifier surfaces as the
// error text rather than panicking — format strings are dev-authored, so a
// mistake should be loud and visible.
fn time_format(nanos: i64, fmt: &str) -> String {
	let ts = match jiff::Timestamp::from_nanosecond(nanos as i64 as i128) {
		Ok(ts) => ts,
		Err(e) => return e.to_string(),
	};
	let zoned = ts.to_zoned(jiff::tz::TimeZone::UTC);
	match jiff::fmt::strtime::format(fmt, &zoned) {
		Ok(s) => s,
		Err(e) => e.to_string(),
	}
}

// Parse an ISO 8601 / RFC 3339 string into a UTC instant. Forgiving: accepts
// a full timestamp (`2026-05-25T14:30:00Z`), a zoneless datetime (assumed
// UTC), or a bare date (midnight UTC).
fn parse_iso(s: &str) -> Result<i64, String> {
	let s = s.trim();
	if let Ok(ts) = s.parse::<jiff::Timestamp>() {
		return Ok(ts.as_nanosecond() as i64);
	}
	if let Ok(dt) = s.parse::<jiff::civil::DateTime>() {
		return dt
			.to_zoned(jiff::tz::TimeZone::UTC)
			.map(|z| z.timestamp().as_nanosecond() as i64)
			.map_err(|e| format!("time: {}", e));
	}
	if let Ok(date) = s.parse::<jiff::civil::Date>() {
		return date
			.to_zoned(jiff::tz::TimeZone::UTC)
			.map(|z| z.timestamp().as_nanosecond() as i64)
			.map_err(|e| format!("time: {}", e));
	}
	Err(format!("time: could not parse `{}` as ISO 8601", s))
}

// Parse with an explicit strftime-style format. If the parsed value carries
// an offset it's honored; otherwise the components are read as UTC.
fn parse_with_format(fmt: &str, input: &str) -> Result<i64, String> {
	let tm = jiff::fmt::strtime::parse(fmt, input).map_err(|e| format!("time: {}", e))?;
	if let Ok(ts) = tm.to_timestamp() {
		return Ok(ts.as_nanosecond() as i64);
	}
	let dt = tm
		.to_datetime()
		.map_err(|e| format!("time: incomplete date/time: {}", e))?;
	dt.to_zoned(jiff::tz::TimeZone::UTC)
		.map(|z| z.timestamp().as_nanosecond() as i64)
		.map_err(|e| format!("time: {}", e))
}

// Calendar-aware shift by whole months and/or years (Jan 31 + 1 month =>
// Feb 28/29). Distinct from duration addition, which is exact nanoseconds.
fn shift_calendar(nanos: i64, months: i64, years: i64) -> Result<i64, String> {
	let ts = jiff::Timestamp::from_nanosecond(nanos as i128).map_err(|e| format!("time: {}", e))?;
	let span = jiff::Span::new()
		.try_months(months)
		.and_then(|s| s.try_years(years))
		.map_err(|e| format!("time: span out of range: {}", e))?;
	let shifted = ts
		.to_zoned(jiff::tz::TimeZone::UTC)
		.checked_add(span)
		.map_err(|e| format!("time: {}", e))?;
	Ok(shifted.timestamp().as_nanosecond() as i64)
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
