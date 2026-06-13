// The playground compile primitive: turn a Pluma source string into a runnable wasm
// module, hex-encoded so it crosses the host boundary (and later the RPC wire) as a
// plain string. Backs `compile.to-wasm-hex` (std/sys/compile). The module is compiled
// for the `Sys` target so its `_entry` runs `main` to completion — the browser's
// `sandbox.run-hex` instantiates it with a console-only import shim.

use super::marshal::{argi, ctx_and_mem, deliver_read_v8, read_str};

/// Largest source the playground accepts — a friendly cap so a runaway paste can't
/// pin a CPU compiling it.
const MAX_SOURCE_BYTES: usize = 64 * 1024;

/// `compile-wasm-hex(src_ptr, src_len, dst, cap) -> len`: compile `src` to a wasm
/// module and deliver its lowercase-hex encoding through `(dst, cap)` (overflow →
/// `io-copyout`). `len < 0` → `err`: the diagnostics/codegen message is stashed in
/// `last_error` (the `__io_result` error channel), rendered plain (no ANSI).
pub(super) fn cb_compile_wasm_hex(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (sp, sl) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (dst, cap) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let source = read_str(scope, mem, sp, sl);

	let n = match compile_to_hex(&source) {
		Ok(hex) => deliver_read_v8(scope, mem, ctx, dst, cap, hex.into_bytes()),
		Err(msg) => {
			ctx.state.last_error = msg;
			-1
		}
	};
	rv.set_int32(n);
}

/// The full pipeline: in-memory source → analyze → ir → WasmGC bytes → hex. Errors
/// (oversize, type errors, codegen faults) come back as a human-readable string.
fn compile_to_hex(source: &str) -> Result<String, String> {
	if source.len() > MAX_SOURCE_BYTES {
		return Err(format!(
			"that source is too large for the playground ({} bytes; the limit is {} KB).",
			source.len(),
			MAX_SOURCE_BYTES / 1024
		));
	}

	// A throwaway compiler rooted at a scratch dir, fed the snippet as the `main`
	// module (so nothing touches disk). Sys target → a plain run-`main` entry.
	let mut compiler =
		compiler::Compiler::for_root_dir(std::env::temp_dir()).with_target(Some(compiler::Target::Sys));
	compiler.set_module_source("main".to_string(), source.as_bytes().to_vec());
	compiler.add_entry_module("main".to_string());

	// `check` returns *every* diagnostic (warnings included) in its `Err`; only
	// error-severity ones block compilation, matching the CLI's run/build behavior.
	if let Err(diags) = compiler.check() {
		if diags.iter().any(compiler::Diagnostic::is_error) {
			return Err(render(&diags, source));
		}
	}

	let program = ir::lower(&compiler).map_err(|m| format!("internal compile error: {m}"))?;
	let bytes = wasm::emit_with_options(
		&program,
		wasm::EmitOptions {
			browser: false,
			..Default::default()
		},
	)
	.map_err(|d| format!("internal codegen error: {}", d.0.join("; ")))?;

	Ok(hex_encode(&bytes))
}

/// Render diagnostics to a plain (no-ANSI) string, feeding the in-memory source back
/// for the code excerpts (the snippet's module never hits disk, so the renderer's
/// own disk read would find nothing).
fn render(diags: &[compiler::Diagnostic], source: &str) -> String {
	compiler::render_diagnostics(
		diags,
		|_path| Some(source.to_string()),
		&compiler::Palette::plain(),
	)
}

/// Lowercase hex, two chars per byte.
fn hex_encode(bytes: &[u8]) -> String {
	let mut s = String::with_capacity(bytes.len() * 2);
	for &b in bytes {
		s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
		s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
	}
	s
}
