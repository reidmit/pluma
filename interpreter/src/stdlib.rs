// Stdlib registration. Each native module shows up to user code as a
// `use core.<name>` import — same surface as user-written modules, but
// implemented in Rust.
//
// One declaration per module — see `regex_module` / `list_module`. The
// `register_compiler` and `register_runtime` entry points walk the same
// description, picking out the types vs. the runtime Builtin tags.
//
// Adding a new native module:
//   1. Implement its builtins in `eval/builtin.rs` (add Builtin enum variants
//      and handlers).
//   2. Write a `<name>_module()` function below describing each def's name,
//      type, and Builtin tag.
//   3. Add it to `modules()`.

use crate::value::{Builtin, Value};
use crate::Interpreter;
use compiler::types::Type;
use compiler::{Compiler, ModuleExports};
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

fn modules() -> Vec<NativeModule> {
	vec![regex_module(), list_module(), math_module()]
}

pub fn register_compiler(compiler: &mut Compiler) {
	for module in modules() {
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

pub fn register_runtime<'ast>(interp: &mut Interpreter<'ast>) {
	for module in modules() {
		let values: HashMap<String, Value<'ast>> = module
			.defs
			.into_iter()
			.map(|d| (d.name.to_string(), Value::Builtin(d.builtin)))
			.collect();
		interp.register_native_module(module.name.to_string(), values);
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

// Polymorphic types use placeholder var IDs (0, 1) — the cross-module
// instantiation path (`Analyzer::instantiate`) freshens every free Var per use
// site, so these IDs are just stand-ins for "the first type variable" and
// "the second", per def.
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
