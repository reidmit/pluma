// Stdlib description, shared between codegen (which exposes types to the
// analyzer) and the VM-side global setup (which puts Builtin values into
// global slots).

use crate::builtin::Builtin;
use crate::value::Value;
use compiler::types::Type;
use compiler::ModuleExports;
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
		io_module(),
	]
}

pub fn register_compiler(compiler: &mut compiler::Compiler) {
	for module in native_modules() {
		let mut values: HashMap<String, Type> = module
			.defs
			.into_iter()
			.map(|d| (d.name.to_string(), d.ty))
			.collect();
		for c in module.constants {
			values.insert(c.name.to_string(), c.ty);
		}
		compiler.register_native_module(
			module.name.to_string(),
			ModuleExports {
				values,
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
			},
			NativeDef {
				name: "is-empty",
				ty: Type::Fun(vec![list_a()], Box::new(Type::Bool)),
				builtin: Builtin::ListIsEmpty,
			},
			NativeDef {
				name: "reverse",
				ty: Type::Fun(vec![list_a()], Box::new(list_a())),
				builtin: Builtin::ListReverse,
			},
			NativeDef {
				name: "concat",
				ty: Type::Fun(vec![list_a(), list_a()], Box::new(list_a())),
				builtin: Builtin::ListConcat,
			},
			NativeDef {
				name: "contains",
				ty: Type::Fun(vec![list_a(), a()], Box::new(Type::Bool)),
				builtin: Builtin::ListContains,
			},
			NativeDef {
				name: "map",
				ty: Type::Fun(
					vec![list_a(), Type::Fun(vec![a()], Box::new(b()))],
					Box::new(list_b()),
				),
				builtin: Builtin::ListMap,
			},
			NativeDef {
				name: "filter",
				ty: Type::Fun(vec![list_a(), pred_a()], Box::new(list_a())),
				builtin: Builtin::ListFilter,
			},
			NativeDef {
				name: "fold",
				ty: Type::Fun(
					vec![list_a(), b(), Type::Fun(vec![b(), a()], Box::new(b()))],
					Box::new(b()),
				),
				builtin: Builtin::ListFold,
			},
			NativeDef {
				name: "each",
				ty: Type::Fun(
					vec![list_a(), Type::Fun(vec![a()], Box::new(b()))],
					Box::new(Type::Nothing),
				),
				builtin: Builtin::ListEach,
			},
			NativeDef {
				name: "head",
				ty: Type::Fun(vec![list_a()], Box::new(option_a())),
				builtin: Builtin::ListHead,
			},
			NativeDef {
				name: "tail",
				ty: Type::Fun(vec![list_a()], Box::new(option_list_a())),
				builtin: Builtin::ListTail,
			},
			NativeDef {
				name: "take",
				ty: Type::Fun(vec![list_a(), Type::Int], Box::new(list_a())),
				builtin: Builtin::ListTake,
			},
			NativeDef {
				name: "drop",
				ty: Type::Fun(vec![list_a(), Type::Int], Box::new(list_a())),
				builtin: Builtin::ListDrop,
			},
			NativeDef {
				name: "find",
				ty: Type::Fun(vec![list_a(), pred_a()], Box::new(option_a())),
				builtin: Builtin::ListFind,
			},
			NativeDef {
				name: "any",
				ty: Type::Fun(vec![list_a(), pred_a()], Box::new(Type::Bool)),
				builtin: Builtin::ListAny,
			},
			NativeDef {
				name: "all",
				ty: Type::Fun(vec![list_a(), pred_a()], Box::new(Type::Bool)),
				builtin: Builtin::ListAll,
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
			},
			NativeDef {
				name: "is-empty",
				ty: str_to_bool(),
				builtin: Builtin::StringIsEmpty,
			},
			NativeDef {
				name: "to-upper",
				ty: str_to_str(),
				builtin: Builtin::StringToUpper,
			},
			NativeDef {
				name: "to-lower",
				ty: str_to_str(),
				builtin: Builtin::StringToLower,
			},
			NativeDef {
				name: "trim",
				ty: str_to_str(),
				builtin: Builtin::StringTrim,
			},
			NativeDef {
				name: "contains",
				ty: two_str_to_bool(),
				builtin: Builtin::StringContains,
			},
			NativeDef {
				name: "starts-with",
				ty: two_str_to_bool(),
				builtin: Builtin::StringStartsWith,
			},
			NativeDef {
				name: "ends-with",
				ty: two_str_to_bool(),
				builtin: Builtin::StringEndsWith,
			},
			NativeDef {
				name: "join",
				ty: Type::Fun(vec![list_str(), Type::String], Box::new(Type::String)),
				builtin: Builtin::StringJoin,
			},
			NativeDef {
				name: "split",
				ty: Type::Fun(vec![Type::String, Type::String], Box::new(list_str())),
				builtin: Builtin::StringSplit,
			},
			NativeDef {
				name: "replace",
				ty: Type::Fun(
					vec![Type::String, Type::String, Type::String],
					Box::new(Type::String),
				),
				builtin: Builtin::StringReplace,
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
			},
			NativeDef {
				name: "print-err",
				ty: Type::Fun(vec![a()], Box::new(a())),
				builtin: Builtin::IoPrintErr,
			},
			NativeDef {
				name: "write",
				ty: Type::Fun(vec![a()], Box::new(a())),
				builtin: Builtin::IoWrite,
			},
			NativeDef {
				name: "write-err",
				ty: Type::Fun(vec![a()], Box::new(a())),
				builtin: Builtin::IoWriteErr,
			},
			NativeDef {
				name: "read-file",
				ty: Type::Fun(vec![Type::String], Box::new(result_str_str())),
				builtin: Builtin::IoReadFile,
			},
			NativeDef {
				name: "write-file",
				ty: Type::Fun(
					vec![Type::String, Type::String],
					Box::new(result_unit_str()),
				),
				builtin: Builtin::IoWriteFile,
			},
			NativeDef {
				name: "file-exists",
				ty: Type::Fun(vec![Type::String], Box::new(Type::Bool)),
				builtin: Builtin::IoFileExists,
			},
			NativeDef {
				name: "args",
				ty: Type::Fun(vec![Type::Nothing], Box::new(Type::List(Box::new(Type::String)))),
				builtin: Builtin::IoArgs,
			},
			NativeDef {
				name: "env",
				ty: Type::Fun(vec![Type::String], Box::new(option_str())),
				builtin: Builtin::IoEnv,
			},
			NativeDef {
				name: "exit",
				ty: Type::Fun(vec![Type::Int], Box::new(a())),
				builtin: Builtin::IoExit,
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
			},
			NativeDef {
				name: "to-int",
				ty: Type::Fun(vec![Type::Float], Box::new(Type::Int)),
				builtin: Builtin::MathToInt,
			},
			NativeDef {
				name: "sqrt",
				ty: float_to_float(),
				builtin: Builtin::MathSqrt,
			},
			NativeDef {
				name: "abs",
				ty: Type::Fun(vec![Type::Int], Box::new(Type::Int)),
				builtin: Builtin::MathAbs,
			},
			NativeDef {
				name: "floor",
				ty: float_to_int(),
				builtin: Builtin::MathFloor,
			},
			NativeDef {
				name: "ceil",
				ty: float_to_int(),
				builtin: Builtin::MathCeil,
			},
			NativeDef {
				name: "round",
				ty: float_to_int(),
				builtin: Builtin::MathRound,
			},
			NativeDef {
				name: "log",
				ty: float_to_float(),
				builtin: Builtin::MathLog,
			},
			NativeDef {
				name: "log10",
				ty: float_to_float(),
				builtin: Builtin::MathLog10,
			},
			NativeDef {
				name: "log2",
				ty: float_to_float(),
				builtin: Builtin::MathLog2,
			},
			NativeDef {
				name: "exp",
				ty: float_to_float(),
				builtin: Builtin::MathExp,
			},
			NativeDef {
				name: "sin",
				ty: float_to_float(),
				builtin: Builtin::MathSin,
			},
			NativeDef {
				name: "cos",
				ty: float_to_float(),
				builtin: Builtin::MathCos,
			},
			NativeDef {
				name: "tan",
				ty: float_to_float(),
				builtin: Builtin::MathTan,
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
