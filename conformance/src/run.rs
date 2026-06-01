// The VM oracle + the shared compile step. Deduped from the (removed)
// wasm_diff.rs / js_diff.rs.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use compiler::{Compiler, Platform};
use ir::IrProgram;

use crate::RunResult;

/// Lower a fixture to IR for a target platform. `Err(msgs)` carries the
/// diagnostic messages — used to tell a *gated* fixture (compiles on `Native`,
/// rejected on `Server`/`Browser`) from a genuine compile-error fixture (fails
/// even on `Native`).
pub(crate) fn compile(dir: &Path, platform: Platform) -> Result<IrProgram, Vec<String>> {
	let mut compiler = match Compiler::from_entry_path(dir.to_str().unwrap().to_string()) {
		Ok(c) => c.with_platform(platform),
		Err(ds) => return Err(ds.iter().map(|d| d.message.clone()).collect()),
	};
	vm::stdlib::register_compiler(&mut compiler);
	if let Err(ds) = compiler.check() {
		return Err(ds.iter().map(|d| d.message.clone()).collect());
	}
	let mut program = ir::lower(&compiler).map_err(|e| vec![e])?;
	// Mirror the `pluma run` VM pipeline so the oracle exercises the same IR the
	// VM actually runs — and so the conformance diff validates that the
	// resolve + inline passes are behavior-neutral against the (un-inlined)
	// deploy backends.
	ir::optimize(&mut program);
	Ok(program)
}

/// Run a compiled VM program, capturing status + stdout — the oracle contract
/// every other backend is diffed against. (`status` = "ok" or "runtime error:
/// <msg>", mirroring `pluma run`.)
pub(crate) fn run_vm(program: vm::Program, stdin: &[u8]) -> RunResult {
	let stdout = Rc::new(RefCell::new(Vec::<u8>::new()));
	// Capture stderr into a discarded buffer too (we only diff status+stdout) so a
	// fixture's `io.print-err` doesn't leak to the harness's own stderr.
	let stderr = Rc::new(RefCell::new(Vec::<u8>::new()));
	let mut vm_instance = vm::VM::new(program)
		.with_stdin(vm::InputSource::Buffer(Rc::new(RefCell::new(
			stdin.to_vec(),
		))))
		.with_stdout(vm::OutputSink::Buffer(stdout.clone()))
		.with_stderr(vm::OutputSink::Buffer(stderr));
	let status = match vm_instance.run() {
		Ok(_) => "ok".to_string(),
		Err(e) => format!("runtime error: {}", e.message),
	};
	RunResult {
		status,
		stdout: String::from_utf8_lossy(&stdout.borrow()).into_owned(),
	}
}
