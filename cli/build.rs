use std::process::Command;

// Bake the version string at compile time. We start from the crate version in
// Cargo.toml (`CARGO_PKG_VERSION`) and, when building inside a git checkout,
// append the short commit SHA — e.g. `0.1.0-a1b2c3d`. Building from a source
// tarball (no git) falls back to the bare crate version.
//
// This lives in the `cli` crate (a leaf binary nothing depends on) on purpose:
// `rerun-if-changed=.git/HEAD` reruns this script on every commit/checkout, and
// the resulting env-var change recompiles the owning crate. Owning it here keeps
// that recompile confined to `cli`; in the hub `compiler` crate it would cascade
// through every downstream crate (ir/wasm/host/lsp/tests) on each commit.
fn main() {
	let base = env!("CARGO_PKG_VERSION");

	let version = match short_sha() {
		Some(sha) => format!("{base}-{sha}"),
		None => base.to_string(),
	};

	println!("cargo:rustc-env=PLUMA_VERSION={version}");

	// Rebuild when HEAD moves so the baked SHA stays current.
	println!("cargo:rerun-if-changed=../.git/HEAD");
	println!("cargo:rerun-if-changed=../.git/refs");
}

fn short_sha() -> Option<String> {
	let output = Command::new("git")
		.args(["rev-parse", "--short", "HEAD"])
		.output()
		.ok()?;

	if !output.status.success() {
		return None;
	}

	let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
	if sha.is_empty() { None } else { Some(sha) }
}
