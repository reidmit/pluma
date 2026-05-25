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
	// No native modules remain: every stdlib module now lives in `.pa`
	// source (see `compiler::stdlib::stdlib_sources`), including `core.dict`
	// — its `where (hash k)` constraints are expressed with the `where`
	// clause on top-level defs. The mechanism below stays for any future
	// module that needs a Rust-defined signature the `.pa` surface can't
	// express.
	Vec::new()
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
