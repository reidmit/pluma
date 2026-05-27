// Lowering: typed AST -> IR.
//
// This is where every backend-independent elaboration lives — the logic
// currently fused into `codegen/src/emit.rs`'s single AST->bytecode walk:
//   * identifier resolution (locals / captures / globals)
//   * closure conversion (explicit capture lists)
//   * dictionary elaboration (trait constraints -> dict params + GetDictMethod)
//   * pattern compilation (`when`/`if is` -> Switch + GetTag/GetPayload)
//   * `defer` edge insertion
//   * async marking (`Function::is_async` + `Await`)
//
// Phase 1.1 ports that elaboration here, function-by-function. So far the two
// standalone pre-passes are ported (enum-table collection + global-slot
// reservation); the per-def expr walk is still `todo!()`, so `lower` is not
// yet complete or wired into `codegen`.

use crate::types::*;
use compiler::ast::DefinitionKind;
use compiler::Compiler;
use std::collections::HashMap;

/// Lower a fully-analyzed program to IR.
///
/// Expects `compiler` to have completed `check()` (every module parsed and
/// analyzed, with inferred types attached to the AST).
///
/// Not yet complete — see the phase plan in `IR.md`. The intended shape:
///   1. collect the enum table from every loaded module      (pre-pass) ✓
///   2. reserve a `GlobalId` per top-level def / alias / instance,
///      after seeding the prelude/native pre-evaluated globals (pre-pass) ✓
///   3. lower each def body to a `Function` (the expr walk)   — todo
///   4. build the entry function and assemble the `IrProgram` — todo
pub fn lower(compiler: &Compiler) -> IrProgram {
	let enums = collect_enums(compiler);

	let mut globals = GlobalTable::new();
	seed_prelude_globals(&mut globals);
	// Native modules currently contribute no globals — `vm::native_modules()`
	// is empty (every stdlib module is `.pa` source). When a Rust-defined
	// native module returns, its defs/constants are seeded here as `PreEval`
	// (a vm-independent encoding: builtin tags + primitive constants).
	reserve_user_globals(&mut globals, compiler);

	let _ = (&enums, &globals);
	todo!("phase 1.1 (cont.): per-def expr walk, thunk emission, entry function")
}

// --------------------------------------------------------------------------
// Pre-pass: enum table.
// --------------------------------------------------------------------------

/// Collect every loaded module's enum definitions into the qualified-name ->
/// variants table. Mirrors `codegen::emit::collect_enum_defs`, run over all
/// modules (including the prelude, which defines `option`/`result`/`ordering`).
fn collect_enums(compiler: &Compiler) -> HashMap<String, Vec<(String, usize)>> {
	let mut out = HashMap::new();
	for (module_name, module) in &compiler.modules {
		let Some(ast) = &module.ast else { continue };
		for def in &ast.body {
			if let DefinitionKind::Enum(enum_node) = &def.kind {
				let qualified = format!("{}.{}", module_name, def.name.name);
				let variants = enum_node
					.variants
					.iter()
					.map(|v| {
						(
							v.name.name.clone(),
							v.params.as_ref().map_or(0, |p| p.len()),
						)
					})
					.collect();
				out.insert(qualified, variants);
			}
		}
	}
	out
}

// --------------------------------------------------------------------------
// Pre-pass: global-slot reservation.
// --------------------------------------------------------------------------

/// Lowering-internal global-slot table. Assigns a `GlobalId` per global in
/// allocation order and tracks each slot's initializer. Prelude/native slots
/// are pre-evaluated up front; user-def slots are `Reserved` until the expr
/// walk fills in their thunk `FuncId`. Becomes `IrProgram::globals` via
/// `finish` at the end of lowering.
struct GlobalTable {
	lookup: HashMap<(String, String), GlobalId>,
	slots: Vec<Slot>,
}

enum Slot {
	PreEvaluated(PreEval),
	/// A user def whose thunk function hasn't been emitted yet.
	Reserved,
	Thunk(FuncId),
}

impl GlobalTable {
	fn new() -> Self {
		Self {
			lookup: HashMap::new(),
			slots: Vec::new(),
		}
	}

	/// Reserve (or return the existing) slot for `(module, name)`. New slots
	/// start `Reserved`. Mirrors `codegen::emit::reserve_global`.
	fn reserve(&mut self, module: &str, name: &str) -> GlobalId {
		let key = (module.to_string(), name.to_string());
		if let Some(&id) = self.lookup.get(&key) {
			return id;
		}
		let id = GlobalId(self.slots.len() as u32);
		self.slots.push(Slot::Reserved);
		self.lookup.insert(key, id);
		id
	}

	/// Reserve a slot and fill it with a pre-evaluated value.
	fn add_pre_evaluated(&mut self, module: &str, name: &str, value: PreEval) -> GlobalId {
		let id = self.reserve(module, name);
		self.slots[id.0 as usize] = Slot::PreEvaluated(value);
		id
	}

	// Wired in the next 1.1 chunk: the expr walk resolves identifiers via
	// `lookup`, records each def's thunk via `set_thunk`, and assembles the
	// final table via `finish`.

	#[allow(dead_code)]
	fn lookup(&self, module: &str, name: &str) -> Option<GlobalId> {
		self
			.lookup
			.get(&(module.to_string(), name.to_string()))
			.copied()
	}

	#[allow(dead_code)]
	fn set_thunk(&mut self, id: GlobalId, func: FuncId) {
		self.slots[id.0 as usize] = Slot::Thunk(func);
	}

	#[allow(dead_code)]
	fn finish(self) -> Vec<GlobalInit> {
		self
			.slots
			.into_iter()
			.map(|slot| match slot {
				Slot::PreEvaluated(v) => GlobalInit::PreEvaluated(v),
				Slot::Thunk(f) => GlobalInit::Thunk(f),
				Slot::Reserved => {
					panic!("global slot left reserved — a def thunk was never assigned")
				}
			})
			.collect()
	}
}

/// Seed the prelude's pre-evaluated globals: the `print`/`debug`/`to-string`
/// builtins and the concrete trait-instance method dictionaries. The dict
/// method order matches each trait's declaration order. Mirrors the prelude
/// block at the top of `codegen::emit::compile`, translated from `vm::Value`
/// into the vm-independent `PreEval`.
fn seed_prelude_globals(g: &mut GlobalTable) {
	let builtin = |tag: &str| PreEval::Builtin(tag.to_string());
	let dict = |tags: &[&str]| PreEval::MethodDict(tags.iter().map(|t| builtin(t)).collect());

	g.add_pre_evaluated("__prelude__", "print", builtin("print"));
	g.add_pre_evaluated("__prelude__", "debug", builtin("debug"));
	g.add_pre_evaluated("__prelude__", "to-string", builtin("to-string"));

	// `numeric`: add, sub, mul, div, negate.
	g.add_pre_evaluated(
		"__prelude__",
		"numeric@int",
		dict(&["int-add", "int-sub", "int-mul", "int-div", "int-negate"]),
	);
	g.add_pre_evaluated(
		"__prelude__",
		"numeric@float",
		dict(&[
			"float-add",
			"float-sub",
			"float-mul",
			"float-div",
			"float-negate",
		]),
	);

	// `ord`: compare.
	g.add_pre_evaluated("__prelude__", "ord@int", dict(&["int-compare"]));
	g.add_pre_evaluated("__prelude__", "ord@float", dict(&["float-compare"]));
	g.add_pre_evaluated("__prelude__", "ord@string", dict(&["string-compare"]));
	g.add_pre_evaluated("__prelude__", "ord@bytes", dict(&["bytes-compare"]));

	// `hash`: hash.
	g.add_pre_evaluated("__prelude__", "hash@int", dict(&["int-hash"]));
	g.add_pre_evaluated("__prelude__", "hash@float", dict(&["float-hash"]));
	g.add_pre_evaluated("__prelude__", "hash@string", dict(&["string-hash"]));
	g.add_pre_evaluated("__prelude__", "hash@bytes", dict(&["bytes-hash"]));
	g.add_pre_evaluated("__prelude__", "hash@bool", dict(&["bool-hash"]));
}

/// Reserve a slot for each user-module top-level value def, alias (its
/// constructor), and trait instance (its method dictionary). Enums and trait
/// declarations are types, not values, so they get no slot. Mirrors the
/// first reservation pass in `codegen::emit::compile`.
fn reserve_user_globals(g: &mut GlobalTable, compiler: &Compiler) {
	for (module_name, module) in &compiler.modules {
		let Some(ast) = &module.ast else { continue };
		for def in &ast.body {
			match &def.kind {
				DefinitionKind::Expr(_) | DefinitionKind::Alias(_) => {
					g.reserve(module_name, &def.name.name);
				}
				DefinitionKind::Enum(_) | DefinitionKind::Trait(_) => {}
				DefinitionKind::Instance(instance) => {
					// The analyzer chose the slot name as `<module>.<trait>@<head>`.
					if let Some((m, n)) = instance.instance_slot_name.rsplit_once('.') {
						g.reserve(m, n);
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;
	use std::sync::atomic::{AtomicU32, Ordering};

	/// Compile a single-module source string in-process and return the
	/// checked compiler. Writes a temp `main.pa` because `from_entry_path` is
	/// the only constructor; the prelude/stdlib are embedded, so no cwd setup
	/// is needed. The temp dir is unique per call (process id + a counter) so
	/// tests running in parallel don't clobber each other's `main.pa`.
	fn check_source(src: &str) -> Compiler {
		static COUNTER: AtomicU32 = AtomicU32::new(0);
		let n = COUNTER.fetch_add(1, Ordering::Relaxed);
		let dir = std::env::temp_dir().join(format!("ir-lower-test-{}-{}", std::process::id(), n));
		std::fs::create_dir_all(&dir).unwrap();
		let path = dir.join("main.pa");
		std::fs::File::create(&path)
			.unwrap()
			.write_all(src.as_bytes())
			.unwrap();
		let mut compiler =
			Compiler::from_entry_path(path.to_str().unwrap().to_string()).expect("from_entry_path");
		compiler.check().expect("check");
		compiler
	}

	#[test]
	fn collects_user_enum_variants() {
		let compiler = check_source("enum color {\n\tred\n\tgreen\n\tblue\n}\n\ndef n = 5\n");
		let enums = collect_enums(&compiler);
		let (_, variants) = enums
			.iter()
			.find(|(k, _)| k.ends_with(".color"))
			.expect("color enum should be collected");
		assert_eq!(
			variants,
			&vec![
				("red".to_string(), 0),
				("green".to_string(), 0),
				("blue".to_string(), 0),
			]
		);
		// Prelude enums come along for free.
		assert!(enums.keys().any(|k| k.ends_with(".option")));
	}

	#[test]
	fn reserves_prelude_and_user_def_globals() {
		let compiler = check_source("enum color {\n\tred\n\tgreen\n}\n\ndef n = 5\n");

		let mut g = GlobalTable::new();
		seed_prelude_globals(&mut g);
		let prelude_count = g.slots.len();
		assert!(g.lookup("__prelude__", "print").is_some());
		assert!(g.lookup("__prelude__", "numeric@int").is_some());

		reserve_user_globals(&mut g, &compiler);
		// The user `def n` got a slot; the `color` enum did not (enums are
		// types, not values).
		assert!(g.slots.len() > prelude_count);
		assert!(g.lookup.keys().any(|(_, name)| name == "n"));
		assert!(!g.lookup.keys().any(|(_, name)| name == "color"));
	}

	#[test]
	fn global_table_dedups_assigns_ids_and_assembles() {
		let mut g = GlobalTable::new();
		let p = g.add_pre_evaluated("m", "print", PreEval::Builtin("print".into()));
		let foo = g.reserve("m", "foo");
		assert_eq!(p, GlobalId(0));
		assert_eq!(foo, GlobalId(1));
		assert_eq!(g.lookup("m", "foo"), Some(GlobalId(1)));
		// Re-reserving the same name returns the existing slot.
		assert_eq!(g.reserve("m", "foo"), foo);

		g.set_thunk(foo, FuncId(7));
		let globals = g.finish();
		assert_eq!(globals.len(), 2);
		assert!(matches!(
			globals[0],
			GlobalInit::PreEvaluated(PreEval::Builtin(_))
		));
		assert!(matches!(globals[1], GlobalInit::Thunk(FuncId(7))));
	}
}
