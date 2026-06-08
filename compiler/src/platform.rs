// Deploy targets and target-tiered module gating.
//
// Pluma compiles from one shared IR to one WasmGC backend, but a `.wasm` runs in
// different host environments — a machine with an operating system underneath
// (servers, CLIs, scripts, batch jobs, desktop) or a web/DOM sandbox. The `Sys`
// and `Web` profiles name that split.
//
// Gating is derived from the module's namespace prefix, not a capability table:
//
//   std.sys.*  → allowed only on the `Sys` profile
//   std.web.*  → allowed only on the `Web` profile
//   std.*      → allowed on both (pure compute / shared host surface)
//
// A `Target` is the chosen deploy profile. Most flows don't choose one: the
// frontend/analysis path, the LSP, `pluma run`/`test`/`check`, and the
// `tests/analyze` suite all compile and run locally under V8 with full host
// capabilities, so they are *ungated* — modeled as `None` (no target). Only
// `pluma build` selects a `Some(target)`: `Web` when `--web` is passed (or the
// client half of a fullstack build), `Sys` otherwise. Gating is enforced at the
// `use` site.

/// A deploy target — the `Sys`/`Web` profile a build is gated against. Not a
/// third "native"/"ungated" variant — ungated mode is `Option::None` (see `gate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
	/// A machine/OS host: servers, CLIs, scripts, batch jobs, desktop.
	/// Grants `std.sys.*`.
	Sys,
	/// A web/DOM sandbox host. Grants `std.web.*`.
	Web,
}

impl Target {
	/// A human-facing name for diagnostics.
	pub fn label(self) -> &'static str {
		match self {
			Target::Sys => "sys",
			Target::Web => "web",
		}
	}

	/// The opposite target — the one whose tier this target does *not* grant.
	/// Used only to phrase the rejection diagnostic.
	fn other(self) -> Target {
		match self {
			Target::Sys => Target::Web,
			Target::Web => Target::Sys,
		}
	}
}

/// The target-specific tier a module name belongs to, if any. `std.sys.*` → `Sys`,
/// `std.web.*` → `Web`, everything else (shared `std.*`, user modules) → `None`.
fn module_tier(module_name: &str) -> Option<Target> {
	if module_name.starts_with("std.sys.") {
		Some(Target::Sys)
	} else if module_name.starts_with("std.web.") {
		Some(Target::Web)
	} else {
		None
	}
}

/// Gate a `use` of `module_name` under an optional deploy `target`. Returns a
/// rejection message if the module's tier doesn't match the active target.
///
/// - `None` (ungated mode): never rejects — the frontend/analysis/run/test path.
/// - `Some(t)`: a `std.<other>.*` module is rejected; shared + matching-tier
///   modules are allowed.
pub fn gate(target: Option<Target>, module_name: &str) -> Option<String> {
	let tier = module_tier(module_name)?;
	match target {
		// Ungated: analyze everything.
		None => None,
		// On a target, only the matching tier's modules are available.
		Some(t) if t == tier => None,
		Some(t) => Some(format!(
			"`{}` is not available on the `{}` target — it is a `{}`-only module.",
			module_name,
			t.label(),
			t.other().label(),
		)),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ungated_allows_everything() {
		for m in ["std.list", "std.sys.io", "std.web.dom", "some.user.module"] {
			assert!(gate(None, m).is_none(), "ungated rejected {m}");
		}
	}

	#[test]
	fn sys_target_allows_sys_and_shared_rejects_web() {
		assert!(gate(Some(Target::Sys), "std.sys.io").is_none());
		assert!(gate(Some(Target::Sys), "std.sys.net").is_none());
		assert!(gate(Some(Target::Sys), "std.list").is_none());
		let rej = gate(Some(Target::Sys), "std.web.dom").expect("web module should be rejected on sys");
		assert!(rej.contains("std.web.dom") && rej.contains("sys"));
	}

	#[test]
	fn web_target_allows_web_and_shared_rejects_sys() {
		assert!(gate(Some(Target::Web), "std.web.dom").is_none());
		assert!(gate(Some(Target::Web), "std.web.render").is_none());
		assert!(gate(Some(Target::Web), "std.list").is_none());
		let rej = gate(Some(Target::Web), "std.sys.io").expect("sys module should be rejected on web");
		assert!(rej.contains("std.sys.io") && rej.contains("web"));
	}

	#[test]
	fn user_modules_never_gated() {
		for t in [None, Some(Target::Sys), Some(Target::Web)] {
			assert!(gate(t, "app.main").is_none());
			assert!(gate(t, "sub.utils").is_none());
		}
	}
}
