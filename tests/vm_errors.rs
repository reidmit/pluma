// VM-internal negative tests: a malformed program must surface a *clean* runtime
// error rather than panicking. These aren't cross-backend cases — they exercise
// the VM's own defensive paths, which the deploy backends reject earlier (e.g.
// `wasm::emit` refuses an unknown builtin tag) or hit differently. So they live
// here as VM unit tests, not as `tests/run-fail/` fixtures (which would force a
// permanent, misleading "skip" in the conformance report).

use compiler::Compiler;

// Referencing a `built-in` tag that doesn't exist type-checks (the analyzer
// trusts builtin names) but must fail at runtime with a clear message — not a
// panic in the VM's builtin dispatch.
#[test]
fn unknown_builtin_tag_is_a_clean_runtime_error() {
	let src = "def whatever :: fun nothing -> nothing = built-in \"no-such-tag\"\n\ndef main = fun {\n\twhatever ()\n}\n";

	let mut compiler = Compiler::for_root_dir(std::env::temp_dir());
	compiler.set_module_source("main".to_string(), src.as_bytes().to_vec());
	compiler.add_entry_module("main".to_string());
	vm::stdlib::register_compiler(&mut compiler);
	compiler
		.check()
		.expect("compiles — the bad builtin tag is only caught at runtime");

	let ir_program = ir::lower(&compiler).expect("lowers");
	let program = codegen::compile_from_ir(&ir_program).expect("codegen");
	// `expect_err` would need `Value: Debug` (it isn't), so match by hand.
	let err = match vm::VM::new(program).run() {
		Ok(_) => panic!("running must error on the unknown builtin"),
		Err(e) => e,
	};

	assert!(
		err.message.contains("unknown builtin") && err.message.contains("no-such-tag"),
		"expected an unknown-builtin runtime error, got: {}",
		err.message
	);
}
