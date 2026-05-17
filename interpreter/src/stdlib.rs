// Stdlib registration. Each native module shows up to user code as a
// `use core.<name>` import — same surface as user-written modules, but
// implemented in Rust.
//
// Adding a new native module:
//   1. Add an entry to `register_compiler` with its `ModuleExports` (types).
//   2. Add an entry to `register_runtime` with its runtime values (Builtin
//      tags pointing at handlers in `eval/builtin.rs`).
// The two sides must agree on def names and shapes — there's no shared
// description today, just two parallel calls.

use crate::value::{Builtin, Value};
use crate::Interpreter;
use compiler::types::Type;
use compiler::{Compiler, ModuleExports};
use std::collections::HashMap;

pub fn register_compiler(compiler: &mut Compiler) {
	let mut regex_values: HashMap<String, Type> = HashMap::new();
	regex_values.insert(
		"matches".into(),
		Type::Fun(vec![Type::Regex, Type::String], Box::new(Type::Bool)),
	);
	compiler.register_native_module(
		"core.regex".into(),
		ModuleExports {
			values: regex_values,
			..Default::default()
		},
	);
}

pub fn register_runtime<'ast>(interp: &mut Interpreter<'ast>) {
	let mut regex_values: HashMap<String, Value<'ast>> = HashMap::new();
	regex_values.insert("matches".into(), Value::Builtin(Builtin::Matches));
	interp.register_native_module("core.regex".into(), regex_values);
}
