// Stdlib description, shared between codegen (which exposes types to the
// analyzer) and the VM-side global setup (which puts Builtin values into
// global slots).

use crate::builtin::Builtin;
use crate::value::Value;
use compiler::types::Type;
use compiler::{ModuleExports, ValueConstraintExport};
use std::collections::HashMap;

pub struct NativeModule {
	pub name: &'static str,
	pub defs: Vec<NativeDef>,
	// Pre-evaluated constants — values, not functions. Loaded as globals
	// the same way functions are, but registered with a concrete Value
	// instead of a Builtin tag so `math.pi` evaluates without a call.
	pub constants: Vec<NativeConstant>,
}

pub struct NativeDef {
	pub name: &'static str,
	pub ty: Type,
	pub builtin: Builtin,
	// Class constraints over tyvars appearing in `ty`. Each `dispatch_var`
	// is a tyvar id used somewhere in `ty`; at every call site the analyzer
	// will resolve a dictionary for that tyvar's resolved type and pass it
	// as a hidden arg before the user-visible args. Empty for the common
	// unconstrained case.
	pub constraints: Vec<NativeConstraint>,
}

pub struct NativeConstraint {
	pub trait_name: &'static str,
	pub dispatch_var: usize,
}

pub struct NativeConstant {
	pub name: &'static str,
	pub ty: Type,
	pub value: Value,
}

pub fn native_modules() -> Vec<NativeModule> {
	vec![
		regex_module(),
		list_module(),
		math_module(),
		string_module(),
		bytes_module(),
		io_module(),
		map_module(),
		ref_module(),
	]
}

pub fn register_compiler(compiler: &mut compiler::Compiler) {
	for module in native_modules() {
		let mut values: HashMap<String, Type> = HashMap::new();
		let mut value_constraints: HashMap<String, Vec<ValueConstraintExport>> = HashMap::new();
		for d in module.defs {
			values.insert(d.name.to_string(), d.ty);
			if !d.constraints.is_empty() {
				let exports: Vec<ValueConstraintExport> = d
					.constraints
					.into_iter()
					.map(|c| ValueConstraintExport {
						trait_name: c.trait_name.to_string(),
						dispatch_var: Type::Var(c.dispatch_var),
					})
					.collect();
				value_constraints.insert(d.name.to_string(), exports);
			}
		}
		for c in module.constants {
			values.insert(c.name.to_string(), c.ty);
		}
		compiler.register_native_module(
			module.name.to_string(),
			ModuleExports {
				values,
				value_constraints,
				..Default::default()
			},
		);
	}
}

fn regex_module() -> NativeModule {
	NativeModule {
		name: "core.regex",
		defs: vec![NativeDef {
			name: "matches",
			ty: Type::Fun(vec![Type::Regex, Type::String], Box::new(Type::Bool)),
			builtin: Builtin::Matches,
			constraints: vec![],
		}],
		constants: vec![],
	}
}

fn list_module() -> NativeModule {
	let a = || Type::Var(0);
	let b = || Type::Var(1);
	let list_a = || Type::List(Box::new(a()));
	let list_b = || Type::List(Box::new(b()));
	let option_a = || Type::Enum("__prelude__.option".to_string(), vec![a()]);
	let option_list_a = || Type::Enum("__prelude__.option".to_string(), vec![list_a()]);
	let pred_a = || Type::Fun(vec![a()], Box::new(Type::Bool));

	NativeModule {
		name: "core.list",
		defs: vec![
			NativeDef {
				name: "length",
				ty: Type::Fun(vec![list_a()], Box::new(Type::Int)),
				builtin: Builtin::ListLength,
				constraints: vec![],
			},
			NativeDef {
				name: "is-empty",
				ty: Type::Fun(vec![list_a()], Box::new(Type::Bool)),
				builtin: Builtin::ListIsEmpty,
				constraints: vec![],
			},
			NativeDef {
				name: "reverse",
				ty: Type::Fun(vec![list_a()], Box::new(list_a())),
				builtin: Builtin::ListReverse,
				constraints: vec![],
			},
			NativeDef {
				name: "concat",
				ty: Type::Fun(vec![list_a(), list_a()], Box::new(list_a())),
				builtin: Builtin::ListConcat,
				constraints: vec![],
			},
			NativeDef {
				name: "contains",
				ty: Type::Fun(vec![list_a(), a()], Box::new(Type::Bool)),
				builtin: Builtin::ListContains,
				constraints: vec![],
			},
			NativeDef {
				name: "map",
				ty: Type::Fun(
					vec![list_a(), Type::Fun(vec![a()], Box::new(b()))],
					Box::new(list_b()),
				),
				builtin: Builtin::ListMap,
				constraints: vec![],
			},
			NativeDef {
				name: "filter",
				ty: Type::Fun(vec![list_a(), pred_a()], Box::new(list_a())),
				builtin: Builtin::ListFilter,
				constraints: vec![],
			},
			NativeDef {
				name: "fold",
				ty: Type::Fun(
					vec![list_a(), b(), Type::Fun(vec![b(), a()], Box::new(b()))],
					Box::new(b()),
				),
				builtin: Builtin::ListFold,
				constraints: vec![],
			},
			NativeDef {
				name: "each",
				ty: Type::Fun(
					vec![list_a(), Type::Fun(vec![a()], Box::new(b()))],
					Box::new(Type::Nothing),
				),
				builtin: Builtin::ListEach,
				constraints: vec![],
			},
			NativeDef {
				name: "head",
				ty: Type::Fun(vec![list_a()], Box::new(option_a())),
				builtin: Builtin::ListHead,
				constraints: vec![],
			},
			NativeDef {
				name: "tail",
				ty: Type::Fun(vec![list_a()], Box::new(option_list_a())),
				builtin: Builtin::ListTail,
				constraints: vec![],
			},
			NativeDef {
				name: "take",
				ty: Type::Fun(vec![list_a(), Type::Int], Box::new(list_a())),
				builtin: Builtin::ListTake,
				constraints: vec![],
			},
			NativeDef {
				name: "drop",
				ty: Type::Fun(vec![list_a(), Type::Int], Box::new(list_a())),
				builtin: Builtin::ListDrop,
				constraints: vec![],
			},
			NativeDef {
				name: "find",
				ty: Type::Fun(vec![list_a(), pred_a()], Box::new(option_a())),
				builtin: Builtin::ListFind,
				constraints: vec![],
			},
			NativeDef {
				name: "any",
				ty: Type::Fun(vec![list_a(), pred_a()], Box::new(Type::Bool)),
				builtin: Builtin::ListAny,
				constraints: vec![],
			},
			NativeDef {
				name: "all",
				ty: Type::Fun(vec![list_a(), pred_a()], Box::new(Type::Bool)),
				builtin: Builtin::ListAll,
				constraints: vec![],
			},
			NativeDef {
				// `sort xs cmp` — `cmp` returns one of the `ordering`
				// variants (lt/eq/gt). Pair with `ord.compare` to sort
				// any list whose elements have an `ord` instance.
				name: "sort",
				ty: Type::Fun(
					vec![
						list_a(),
						Type::Fun(
							vec![a(), a()],
							Box::new(Type::Enum("__prelude__.ordering".to_string(), vec![])),
						),
					],
					Box::new(list_a()),
				),
				builtin: Builtin::ListSort,
				constraints: vec![],
			},
		],
		constants: vec![],
	}
}

fn string_module() -> NativeModule {
	let str_to_str = || Type::Fun(vec![Type::String], Box::new(Type::String));
	let str_to_int = || Type::Fun(vec![Type::String], Box::new(Type::Int));
	let str_to_bool = || Type::Fun(vec![Type::String], Box::new(Type::Bool));
	let two_str_to_bool = || Type::Fun(vec![Type::String, Type::String], Box::new(Type::Bool));
	let list_str = || Type::List(Box::new(Type::String));

	NativeModule {
		name: "core.string",
		defs: vec![
			NativeDef {
				name: "length",
				ty: str_to_int(),
				builtin: Builtin::StringLength,
				constraints: vec![],
			},
			NativeDef {
				name: "is-empty",
				ty: str_to_bool(),
				builtin: Builtin::StringIsEmpty,
				constraints: vec![],
			},
			NativeDef {
				name: "to-upper",
				ty: str_to_str(),
				builtin: Builtin::StringToUpper,
				constraints: vec![],
			},
			NativeDef {
				name: "to-lower",
				ty: str_to_str(),
				builtin: Builtin::StringToLower,
				constraints: vec![],
			},
			NativeDef {
				name: "trim",
				ty: str_to_str(),
				builtin: Builtin::StringTrim,
				constraints: vec![],
			},
			NativeDef {
				name: "contains",
				ty: two_str_to_bool(),
				builtin: Builtin::StringContains,
				constraints: vec![],
			},
			NativeDef {
				name: "starts-with",
				ty: two_str_to_bool(),
				builtin: Builtin::StringStartsWith,
				constraints: vec![],
			},
			NativeDef {
				name: "ends-with",
				ty: two_str_to_bool(),
				builtin: Builtin::StringEndsWith,
				constraints: vec![],
			},
			NativeDef {
				name: "join",
				ty: Type::Fun(vec![list_str(), Type::String], Box::new(Type::String)),
				builtin: Builtin::StringJoin,
				constraints: vec![],
			},
			NativeDef {
				name: "split",
				ty: Type::Fun(vec![Type::String, Type::String], Box::new(list_str())),
				builtin: Builtin::StringSplit,
				constraints: vec![],
			},
			NativeDef {
				name: "replace",
				ty: Type::Fun(
					vec![Type::String, Type::String, Type::String],
					Box::new(Type::String),
				),
				builtin: Builtin::StringReplace,
				constraints: vec![],
			},
			NativeDef {
				name: "to-int",
				ty: Type::Fun(
					vec![Type::String],
					Box::new(Type::Enum(
						"__prelude__.result".to_string(),
						vec![Type::Int, Type::String],
					)),
				),
				builtin: Builtin::StringToInt,
				constraints: vec![],
			},
			NativeDef {
				name: "to-float",
				ty: Type::Fun(
					vec![Type::String],
					Box::new(Type::Enum(
						"__prelude__.result".to_string(),
						vec![Type::Float, Type::String],
					)),
				),
				builtin: Builtin::StringToFloat,
				constraints: vec![],
			},
			NativeDef {
				name: "to-bytes",
				ty: Type::Fun(vec![Type::String], Box::new(Type::Bytes)),
				builtin: Builtin::StringToBytes,
				constraints: vec![],
			},
		],
		constants: vec![],
	}
}

fn bytes_module() -> NativeModule {
	let bytes_to_bytes = || Type::Fun(vec![Type::Bytes], Box::new(Type::Bytes));
	let bytes_to_int = || Type::Fun(vec![Type::Bytes], Box::new(Type::Int));
	let bytes_to_bool = || Type::Fun(vec![Type::Bytes], Box::new(Type::Bool));
	let two_bytes_to_bool = || Type::Fun(vec![Type::Bytes, Type::Bytes], Box::new(Type::Bool));
	let list_bytes = || Type::List(Box::new(Type::Bytes));
	let option_int = || Type::Enum("__prelude__.option".to_string(), vec![Type::Int]);
	let result_bytes_str = || {
		Type::Enum(
			"__prelude__.result".to_string(),
			vec![Type::Bytes, Type::String],
		)
	};
	let result_str_str = || {
		Type::Enum(
			"__prelude__.result".to_string(),
			vec![Type::String, Type::String],
		)
	};

	NativeModule {
		name: "core.bytes",
		defs: vec![
			NativeDef {
				name: "length",
				ty: bytes_to_int(),
				builtin: Builtin::BytesLength,
				constraints: vec![],
			},
			NativeDef {
				name: "is-empty",
				ty: bytes_to_bool(),
				builtin: Builtin::BytesIsEmpty,
				constraints: vec![],
			},
			NativeDef {
				name: "at",
				ty: Type::Fun(vec![Type::Bytes, Type::Int], Box::new(option_int())),
				builtin: Builtin::BytesAt,
				constraints: vec![],
			},
			NativeDef {
				name: "concat",
				ty: Type::Fun(vec![Type::Bytes, Type::Bytes], Box::new(Type::Bytes)),
				builtin: Builtin::BytesConcat,
				constraints: vec![],
			},
			NativeDef {
				name: "slice",
				ty: Type::Fun(
					vec![Type::Bytes, Type::Int, Type::Int],
					Box::new(Type::Bytes),
				),
				builtin: Builtin::BytesSlice,
				constraints: vec![],
			},
			NativeDef {
				name: "contains",
				ty: two_bytes_to_bool(),
				builtin: Builtin::BytesContains,
				constraints: vec![],
			},
			NativeDef {
				name: "starts-with",
				ty: two_bytes_to_bool(),
				builtin: Builtin::BytesStartsWith,
				constraints: vec![],
			},
			NativeDef {
				name: "ends-with",
				ty: two_bytes_to_bool(),
				builtin: Builtin::BytesEndsWith,
				constraints: vec![],
			},
			NativeDef {
				name: "repeat",
				ty: Type::Fun(vec![Type::Bytes, Type::Int], Box::new(Type::Bytes)),
				builtin: Builtin::BytesRepeat,
				constraints: vec![],
			},
			NativeDef {
				name: "reverse",
				ty: bytes_to_bytes(),
				builtin: Builtin::BytesReverse,
				constraints: vec![],
			},
			NativeDef {
				name: "to-list",
				ty: Type::Fun(vec![Type::Bytes], Box::new(Type::List(Box::new(Type::Int)))),
				builtin: Builtin::BytesToList,
				constraints: vec![],
			},
			NativeDef {
				name: "from-list",
				ty: Type::Fun(
					vec![Type::List(Box::new(Type::Int))],
					Box::new(result_bytes_str()),
				),
				builtin: Builtin::BytesFromList,
				constraints: vec![],
			},
			NativeDef {
				name: "join",
				ty: Type::Fun(vec![list_bytes(), Type::Bytes], Box::new(Type::Bytes)),
				builtin: Builtin::BytesJoin,
				constraints: vec![],
			},
			NativeDef {
				name: "split",
				ty: Type::Fun(vec![Type::Bytes, Type::Bytes], Box::new(list_bytes())),
				builtin: Builtin::BytesSplit,
				constraints: vec![],
			},
			NativeDef {
				name: "to-string",
				ty: Type::Fun(vec![Type::Bytes], Box::new(result_str_str())),
				builtin: Builtin::BytesToString,
				constraints: vec![],
			},
		],
		constants: vec![],
	}
}

fn io_module() -> NativeModule {
	let a = || Type::Var(0);
	let result_unit_str = || {
		Type::Enum(
			"__prelude__.result".to_string(),
			vec![Type::Nothing, Type::String],
		)
	};
	let result_str_str = || {
		Type::Enum(
			"__prelude__.result".to_string(),
			vec![Type::String, Type::String],
		)
	};
	let option_str = || Type::Enum("__prelude__.option".to_string(), vec![Type::String]);

	NativeModule {
		name: "core.io",
		defs: vec![
			NativeDef {
				name: "print",
				ty: Type::Fun(vec![a()], Box::new(a())),
				builtin: Builtin::IoPrint,
				constraints: vec![],
			},
			NativeDef {
				name: "print-err",
				ty: Type::Fun(vec![a()], Box::new(a())),
				builtin: Builtin::IoPrintErr,
				constraints: vec![],
			},
			NativeDef {
				name: "write",
				ty: Type::Fun(vec![a()], Box::new(a())),
				builtin: Builtin::IoWrite,
				constraints: vec![],
			},
			NativeDef {
				name: "write-err",
				ty: Type::Fun(vec![a()], Box::new(a())),
				builtin: Builtin::IoWriteErr,
				constraints: vec![],
			},
			NativeDef {
				name: "read",
				ty: Type::Fun(vec![Type::Nothing], Box::new(result_str_str())),
				builtin: Builtin::IoRead,
				constraints: vec![],
			},
			NativeDef {
				name: "read-all",
				ty: Type::Fun(vec![Type::Nothing], Box::new(result_str_str())),
				builtin: Builtin::IoReadAll,
				constraints: vec![],
			},
			NativeDef {
				name: "read-file",
				ty: Type::Fun(vec![Type::String], Box::new(result_str_str())),
				builtin: Builtin::IoReadFile,
				constraints: vec![],
			},
			NativeDef {
				name: "write-file",
				ty: Type::Fun(
					vec![Type::String, Type::String],
					Box::new(result_unit_str()),
				),
				builtin: Builtin::IoWriteFile,
				constraints: vec![],
			},
			NativeDef {
				name: "append-file",
				ty: Type::Fun(
					vec![Type::String, Type::String],
					Box::new(result_unit_str()),
				),
				builtin: Builtin::IoAppendFile,
				constraints: vec![],
			},
			NativeDef {
				name: "file-exists",
				ty: Type::Fun(vec![Type::String], Box::new(Type::Bool)),
				builtin: Builtin::IoFileExists,
				constraints: vec![],
			},
			NativeDef {
				name: "delete-file",
				ty: Type::Fun(vec![Type::String], Box::new(result_unit_str())),
				builtin: Builtin::IoDeleteFile,
				constraints: vec![],
			},
			NativeDef {
				name: "args",
				ty: Type::Fun(
					vec![Type::Nothing],
					Box::new(Type::List(Box::new(Type::String))),
				),
				builtin: Builtin::IoArgs,
				constraints: vec![],
			},
			NativeDef {
				name: "env",
				ty: Type::Fun(vec![Type::String], Box::new(option_str())),
				builtin: Builtin::IoEnv,
				constraints: vec![],
			},
			NativeDef {
				name: "exit",
				ty: Type::Fun(vec![Type::Int], Box::new(a())),
				builtin: Builtin::IoExit,
				constraints: vec![],
			},
			NativeDef {
				name: "read-all-bytes",
				ty: Type::Fun(
					vec![Type::Nothing],
					Box::new(Type::Enum(
						"__prelude__.result".to_string(),
						vec![Type::Bytes, Type::String],
					)),
				),
				builtin: Builtin::IoReadAllBytes,
				constraints: vec![],
			},
			NativeDef {
				name: "read-file-bytes",
				ty: Type::Fun(
					vec![Type::String],
					Box::new(Type::Enum(
						"__prelude__.result".to_string(),
						vec![Type::Bytes, Type::String],
					)),
				),
				builtin: Builtin::IoReadFileBytes,
				constraints: vec![],
			},
			NativeDef {
				name: "write-file-bytes",
				ty: Type::Fun(
					vec![Type::String, Type::Bytes],
					Box::new(result_unit_str()),
				),
				builtin: Builtin::IoWriteFileBytes,
				constraints: vec![],
			},
			NativeDef {
				name: "append-file-bytes",
				ty: Type::Fun(
					vec![Type::String, Type::Bytes],
					Box::new(result_unit_str()),
				),
				builtin: Builtin::IoAppendFileBytes,
				constraints: vec![],
			},
			NativeDef {
				name: "write-bytes",
				ty: Type::Fun(vec![Type::Bytes], Box::new(Type::Bytes)),
				builtin: Builtin::IoWriteBytes,
				constraints: vec![],
			},
			NativeDef {
				name: "write-err-bytes",
				ty: Type::Fun(vec![Type::Bytes], Box::new(Type::Bytes)),
				builtin: Builtin::IoWriteErrBytes,
				constraints: vec![],
			},
		],
		constants: vec![],
	}
}

fn math_module() -> NativeModule {
	let float_to_float = || Type::Fun(vec![Type::Float], Box::new(Type::Float));
	let float_to_int = || Type::Fun(vec![Type::Float], Box::new(Type::Int));
	NativeModule {
		name: "core.math",
		defs: vec![
			NativeDef {
				name: "to-float",
				ty: Type::Fun(vec![Type::Int], Box::new(Type::Float)),
				builtin: Builtin::MathToFloat,
				constraints: vec![],
			},
			NativeDef {
				name: "to-int",
				ty: Type::Fun(vec![Type::Float], Box::new(Type::Int)),
				builtin: Builtin::MathToInt,
				constraints: vec![],
			},
			NativeDef {
				name: "sqrt",
				ty: float_to_float(),
				builtin: Builtin::MathSqrt,
				constraints: vec![],
			},
			NativeDef {
				name: "abs",
				ty: Type::Fun(vec![Type::Int], Box::new(Type::Int)),
				builtin: Builtin::MathAbs,
				constraints: vec![],
			},
			NativeDef {
				name: "floor",
				ty: float_to_int(),
				builtin: Builtin::MathFloor,
				constraints: vec![],
			},
			NativeDef {
				name: "ceil",
				ty: float_to_int(),
				builtin: Builtin::MathCeil,
				constraints: vec![],
			},
			NativeDef {
				name: "round",
				ty: float_to_int(),
				builtin: Builtin::MathRound,
				constraints: vec![],
			},
			NativeDef {
				name: "log",
				ty: float_to_float(),
				builtin: Builtin::MathLog,
				constraints: vec![],
			},
			NativeDef {
				name: "log10",
				ty: float_to_float(),
				builtin: Builtin::MathLog10,
				constraints: vec![],
			},
			NativeDef {
				name: "log2",
				ty: float_to_float(),
				builtin: Builtin::MathLog2,
				constraints: vec![],
			},
			NativeDef {
				name: "exp",
				ty: float_to_float(),
				builtin: Builtin::MathExp,
				constraints: vec![],
			},
			NativeDef {
				name: "sin",
				ty: float_to_float(),
				builtin: Builtin::MathSin,
				constraints: vec![],
			},
			NativeDef {
				name: "cos",
				ty: float_to_float(),
				builtin: Builtin::MathCos,
				constraints: vec![],
			},
			NativeDef {
				name: "tan",
				ty: float_to_float(),
				builtin: Builtin::MathTan,
				constraints: vec![],
			},
		],
		constants: vec![
			NativeConstant {
				name: "pi",
				ty: Type::Float,
				value: Value::Float(std::f64::consts::PI),
			},
			NativeConstant {
				name: "e",
				ty: Type::Float,
				value: Value::Float(std::f64::consts::E),
			},
		],
	}
}

fn ref_module() -> NativeModule {
	let a = || Type::Var(0);
	let ref_a = || Type::Ref(Box::new(a()));
	NativeModule {
		name: "core.ref",
		defs: vec![
			NativeDef {
				name: "new",
				ty: Type::Fun(vec![a()], Box::new(ref_a())),
				builtin: Builtin::RefNew,
				constraints: vec![],
			},
			NativeDef {
				name: "get",
				ty: Type::Fun(vec![ref_a()], Box::new(a())),
				builtin: Builtin::RefGet,
				constraints: vec![],
			},
			NativeDef {
				name: "set",
				ty: Type::Fun(vec![ref_a(), a()], Box::new(Type::Nothing)),
				builtin: Builtin::RefSet,
				constraints: vec![],
			},
			NativeDef {
				// `update r f` — read, apply f, write back. Returns nothing
				// so call sites don't try to chain on the new value; ask
				// `get` if you need it.
				name: "update",
				ty: Type::Fun(
					vec![ref_a(), Type::Fun(vec![a()], Box::new(a()))],
					Box::new(Type::Nothing),
				),
				builtin: Builtin::RefUpdate,
				constraints: vec![],
			},
		],
		constants: vec![],
	}
}

fn map_module() -> NativeModule {
	// Tyvar ids: k = 0, v = 1, b = 2. The dispatch_var for `where (hash k)`
	// is the same id (0) used in the function's signature — see how
	// `value_constraints` exports work in compiler/src/module.rs.
	let k = || Type::Var(0);
	let v = || Type::Var(1);
	let b = || Type::Var(2);
	let map_kv = || Type::Map(Box::new(k()), Box::new(v()));
	let option_v = || Type::Enum("__prelude__.option".to_string(), vec![v()]);
	let entry_kv = || Type::Tuple(vec![k(), v()]);
	let list_entries = || Type::List(Box::new(entry_kv()));
	let hash_k = || NativeConstraint {
		trait_name: "hash",
		dispatch_var: 0,
	};
	NativeModule {
		name: "core.map",
		defs: vec![
			NativeDef {
				name: "empty",
				ty: Type::Fun(vec![Type::Nothing], Box::new(map_kv())),
				builtin: Builtin::MapEmpty,
				constraints: vec![],
			},
			NativeDef {
				name: "insert",
				ty: Type::Fun(vec![map_kv(), k(), v()], Box::new(map_kv())),
				builtin: Builtin::MapInsert,
				constraints: vec![hash_k()],
			},
			NativeDef {
				name: "lookup",
				ty: Type::Fun(vec![map_kv(), k()], Box::new(option_v())),
				builtin: Builtin::MapLookup,
				constraints: vec![hash_k()],
			},
			NativeDef {
				name: "remove",
				ty: Type::Fun(vec![map_kv(), k()], Box::new(map_kv())),
				builtin: Builtin::MapRemove,
				constraints: vec![hash_k()],
			},
			NativeDef {
				name: "contains-key",
				ty: Type::Fun(vec![map_kv(), k()], Box::new(Type::Bool)),
				builtin: Builtin::MapContainsKey,
				constraints: vec![hash_k()],
			},
			NativeDef {
				name: "size",
				ty: Type::Fun(vec![map_kv()], Box::new(Type::Int)),
				builtin: Builtin::MapSize,
				constraints: vec![],
			},
			NativeDef {
				name: "keys",
				ty: Type::Fun(vec![map_kv()], Box::new(Type::List(Box::new(k())))),
				builtin: Builtin::MapKeys,
				constraints: vec![],
			},
			NativeDef {
				name: "values",
				ty: Type::Fun(vec![map_kv()], Box::new(Type::List(Box::new(v())))),
				builtin: Builtin::MapValues,
				constraints: vec![],
			},
			NativeDef {
				name: "entries",
				ty: Type::Fun(vec![map_kv()], Box::new(list_entries())),
				builtin: Builtin::MapEntries,
				constraints: vec![],
			},
			NativeDef {
				name: "from-entries",
				ty: Type::Fun(vec![list_entries()], Box::new(map_kv())),
				builtin: Builtin::MapFromEntries,
				constraints: vec![hash_k()],
			},
			NativeDef {
				name: "merge",
				ty: Type::Fun(vec![map_kv(), map_kv()], Box::new(map_kv())),
				builtin: Builtin::MapMerge,
				constraints: vec![hash_k()],
			},
			NativeDef {
				// `map m fn` — fn is applied to each value; keys are unchanged.
				name: "map",
				ty: Type::Fun(
					vec![
						Type::Map(Box::new(k()), Box::new(v())),
						Type::Fun(vec![v()], Box::new(b())),
					],
					Box::new(Type::Map(Box::new(k()), Box::new(b()))),
				),
				builtin: Builtin::MapMap,
				constraints: vec![],
			},
			NativeDef {
				name: "filter",
				ty: Type::Fun(
					vec![map_kv(), Type::Fun(vec![k(), v()], Box::new(Type::Bool))],
					Box::new(map_kv()),
				),
				builtin: Builtin::MapFilter,
				constraints: vec![],
			},
			NativeDef {
				name: "fold",
				ty: Type::Fun(
					vec![map_kv(), b(), Type::Fun(vec![b(), k(), v()], Box::new(b()))],
					Box::new(b()),
				),
				builtin: Builtin::MapFold,
				constraints: vec![],
			},
		],
		constants: vec![],
	}
}
