// Stdlib description, shared between codegen (which exposes types to the
// analyzer) and the VM-side global setup (which puts builtin tags into
// global slots).

use crate::value::Value;
use compiler::types::Type;
use compiler::{ModuleExports, ValueConstraintExport};
use std::collections::HashMap;

pub struct NativeModule {
	pub name: &'static str,
	pub defs: Vec<NativeDef>,
	// Pre-evaluated constants — values, not functions. Loaded as globals
	// the same way functions are, but registered with a concrete Value
	// instead of a builtin tag so `math.pi` evaluates without a call.
	pub constants: Vec<NativeConstant>,
}

pub struct NativeDef {
	pub name: &'static str,
	pub ty: Type,
	pub builtin_tag: &'static str,
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
	// Remaining native modules: `core.map`'s defs carry `where (hash k)`
	// constraints whose expression-on-Pluma side awaits `where` syntax
	// on top-level def annotations. Everything else lives in the `.pa`
	// stdlib (see `compiler::stdlib::stdlib_sources`).
	vec![map_module()]
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
				builtin_tag: "map-empty",
				constraints: vec![],
			},
			NativeDef {
				name: "insert",
				ty: Type::Fun(vec![map_kv(), k(), v()], Box::new(map_kv())),
				builtin_tag: "map-insert",
				constraints: vec![hash_k()],
			},
			NativeDef {
				name: "lookup",
				ty: Type::Fun(vec![map_kv(), k()], Box::new(option_v())),
				builtin_tag: "map-lookup",
				constraints: vec![hash_k()],
			},
			NativeDef {
				name: "remove",
				ty: Type::Fun(vec![map_kv(), k()], Box::new(map_kv())),
				builtin_tag: "map-remove",
				constraints: vec![hash_k()],
			},
			NativeDef {
				name: "contains-key",
				ty: Type::Fun(vec![map_kv(), k()], Box::new(Type::Bool)),
				builtin_tag: "map-contains-key",
				constraints: vec![hash_k()],
			},
			NativeDef {
				name: "size",
				ty: Type::Fun(vec![map_kv()], Box::new(Type::Int)),
				builtin_tag: "map-size",
				constraints: vec![],
			},
			NativeDef {
				name: "keys",
				ty: Type::Fun(vec![map_kv()], Box::new(Type::List(Box::new(k())))),
				builtin_tag: "map-keys",
				constraints: vec![],
			},
			NativeDef {
				name: "values",
				ty: Type::Fun(vec![map_kv()], Box::new(Type::List(Box::new(v())))),
				builtin_tag: "map-values",
				constraints: vec![],
			},
			NativeDef {
				name: "entries",
				ty: Type::Fun(vec![map_kv()], Box::new(list_entries())),
				builtin_tag: "map-entries",
				constraints: vec![],
			},
			NativeDef {
				name: "from-entries",
				ty: Type::Fun(vec![list_entries()], Box::new(map_kv())),
				builtin_tag: "map-from-entries",
				constraints: vec![hash_k()],
			},
			NativeDef {
				name: "merge",
				ty: Type::Fun(vec![map_kv(), map_kv()], Box::new(map_kv())),
				builtin_tag: "map-merge",
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
				builtin_tag: "map-map",
				constraints: vec![],
			},
			NativeDef {
				name: "filter",
				ty: Type::Fun(
					vec![map_kv(), Type::Fun(vec![k(), v()], Box::new(Type::Bool))],
					Box::new(map_kv()),
				),
				builtin_tag: "map-filter",
				constraints: vec![],
			},
			NativeDef {
				name: "fold",
				ty: Type::Fun(
					vec![map_kv(), b(), Type::Fun(vec![b(), k(), v()], Box::new(b()))],
					Box::new(b()),
				),
				builtin_tag: "map-fold",
				constraints: vec![],
			},
		],
		constants: vec![],
	}
}
