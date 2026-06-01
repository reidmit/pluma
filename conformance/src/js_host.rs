// The JS backend runner: write the emitted module to a temp file and run it
// under `node`, reconstructing the (status, stdout) contract. Migrated from the
// (removed) js_diff.rs. There is no in-process JS engine, so this is the only
// JS execution path (and it includes node process startup).

use std::io::Write;
use std::process::Command;

use crate::RunResult;

/// Locate a usable `node` (honoring `$PLUMA_NODE`), or `None` if absent.
pub(crate) fn find_node() -> Option<String> {
	let node = std::env::var("PLUMA_NODE").unwrap_or_else(|_| "node".to_string());
	Command::new(&node)
		.arg("--version")
		.output()
		.ok()
		.filter(|o| o.status.success())
		.map(|_| node)
}

/// Run an emitted JS module under node. stdout is the program's stdout; the
/// status is "ok" on a zero exit, else the last stderr line (the runner writes
/// `runtime error: <msg>` there). `None` if node can't be launched.
pub(crate) fn run_node(node: &str, source: &str, name: &str) -> Option<RunResult> {
	let path = std::env::temp_dir().join(format!("pluma_conf_{name}.js"));
	std::fs::File::create(&path)
		.and_then(|mut f| f.write_all(source.as_bytes()))
		.expect("write temp JS");
	let out = Command::new(node).arg(&path).output().ok()?;
	let _ = std::fs::remove_file(&path);
	let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
	let status = if out.status.success() {
		"ok".to_string()
	} else {
		let stderr = String::from_utf8_lossy(&out.stderr);
		stderr
			.lines()
			.rev()
			.find(|l| !l.trim().is_empty())
			.unwrap_or("runtime error")
			.to_string()
	};
	Some(RunResult { status, stdout })
}
