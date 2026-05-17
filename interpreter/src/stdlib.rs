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
	compiler.register_native_module("core.regex".into(), regex_exports());
	compiler.register_native_module("core.list".into(), list_exports());
}

pub fn register_runtime<'ast>(interp: &mut Interpreter<'ast>) {
	let mut regex_values: HashMap<String, Value<'ast>> = HashMap::new();
	regex_values.insert("matches".into(), Value::Builtin(Builtin::Matches));
	interp.register_native_module("core.regex".into(), regex_values);

	let mut list_values: HashMap<String, Value<'ast>> = HashMap::new();
	list_values.insert("length".into(), Value::Builtin(Builtin::ListLength));
	list_values.insert("is-empty".into(), Value::Builtin(Builtin::ListIsEmpty));
	list_values.insert("reverse".into(), Value::Builtin(Builtin::ListReverse));
	list_values.insert("concat".into(), Value::Builtin(Builtin::ListConcat));
	list_values.insert("contains".into(), Value::Builtin(Builtin::ListContains));
	list_values.insert("map".into(), Value::Builtin(Builtin::ListMap));
	list_values.insert("filter".into(), Value::Builtin(Builtin::ListFilter));
	list_values.insert("fold".into(), Value::Builtin(Builtin::ListFold));
	list_values.insert("each".into(), Value::Builtin(Builtin::ListEach));
	interp.register_native_module("core.list".into(), list_values);
}

fn regex_exports() -> ModuleExports {
	let mut values: HashMap<String, Type> = HashMap::new();
	values.insert(
		"matches".into(),
		Type::Fun(vec![Type::Regex, Type::String], Box::new(Type::Bool)),
	);
	ModuleExports {
		values,
		..Default::default()
	}
}

// Polymorphic types use fresh-looking var IDs (0, 1) — the cross-module
// instantiation path (`Analyzer::instantiate`) freshens every free Var per use
// site, so these IDs are just placeholders for "the first type variable" and
// "the second."
fn list_exports() -> ModuleExports {
	let a = || Type::Var(0);
	let b = || Type::Var(1);
	let list_a = || Type::List(Box::new(a()));
	let list_b = || Type::List(Box::new(b()));

	let mut values: HashMap<String, Type> = HashMap::new();

	// length: list a -> int
	values.insert(
		"length".into(),
		Type::Fun(vec![list_a()], Box::new(Type::Int)),
	);

	// is-empty: list a -> bool
	values.insert(
		"is-empty".into(),
		Type::Fun(vec![list_a()], Box::new(Type::Bool)),
	);

	// reverse: list a -> list a
	values.insert(
		"reverse".into(),
		Type::Fun(vec![list_a()], Box::new(list_a())),
	);

	// concat: list a -> list a -> list a
	values.insert(
		"concat".into(),
		Type::Fun(vec![list_a(), list_a()], Box::new(list_a())),
	);

	// contains: list a -> a -> bool
	values.insert(
		"contains".into(),
		Type::Fun(vec![list_a(), a()], Box::new(Type::Bool)),
	);

	// map: list a -> (a -> b) -> list b
	values.insert(
		"map".into(),
		Type::Fun(
			vec![list_a(), Type::Fun(vec![a()], Box::new(b()))],
			Box::new(list_b()),
		),
	);

	// filter: list a -> (a -> bool) -> list a
	values.insert(
		"filter".into(),
		Type::Fun(
			vec![list_a(), Type::Fun(vec![a()], Box::new(Type::Bool))],
			Box::new(list_a()),
		),
	);

	// fold: list a -> b -> (b -> a -> b) -> b
	values.insert(
		"fold".into(),
		Type::Fun(
			vec![list_a(), b(), Type::Fun(vec![b(), a()], Box::new(b()))],
			Box::new(b()),
		),
	);

	// each: list a -> (a -> b) -> nothing  (b is discarded)
	values.insert(
		"each".into(),
		Type::Fun(
			vec![list_a(), Type::Fun(vec![a()], Box::new(b()))],
			Box::new(Type::Nothing),
		),
	);

	ModuleExports {
		values,
		..Default::default()
	}
}
