// Stdlib description, shared between codegen (which exposes types to the
// analyzer) and the VM-side global setup (which puts Builtin values into
// global slots).

use crate::builtin::Builtin;
use compiler::types::Type;
use compiler::ModuleExports;
use std::collections::HashMap;

pub struct NativeModule {
	pub name: &'static str,
	pub defs: Vec<NativeDef>,
}

pub struct NativeDef {
	pub name: &'static str,
	pub ty: Type,
	pub builtin: Builtin,
}

pub fn native_modules() -> Vec<NativeModule> {
	vec![regex_module(), list_module(), math_module()]
}

pub fn register_compiler(compiler: &mut compiler::Compiler) {
	for module in native_modules() {
		let values: HashMap<String, Type> = module
			.defs
			.into_iter()
			.map(|d| (d.name.to_string(), d.ty))
			.collect();
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
	}
}

fn list_module() -> NativeModule {
	let a = || Type::Var(0);
	let b = || Type::Var(1);
	let list_a = || Type::List(Box::new(a()));
	let list_b = || Type::List(Box::new(b()));

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
				ty: Type::Fun(
					vec![list_a(), Type::Fun(vec![a()], Box::new(Type::Bool))],
					Box::new(list_a()),
				),
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
		],
	}
}

fn math_module() -> NativeModule {
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
				ty: Type::Fun(vec![Type::Float], Box::new(Type::Float)),
				builtin: Builtin::MathSqrt,
			},
			NativeDef {
				name: "abs",
				ty: Type::Fun(vec![Type::Int], Box::new(Type::Int)),
				builtin: Builtin::MathAbs,
			},
		],
	}
}
