// Target platforms and host capabilities.
//
// Pluma compiles from one shared IR to one WasmGC backend, but a `.wasm` runs in
// different host environments — a server/machine or a browser. A `Platform` is a
// host-capability *profile*: it decides which stdlib modules a program may `use`
// and (downstream) which host imports the module declares. The bytecode VM, used
// for `pluma run`, tests, and scripting, is the `Native` profile — it provides
// every capability, so nothing is ever gated on it and existing behavior is
// unchanged.
//
// Gating is a static table: each gated module declares the capabilities it needs
// (`MODULE_CAPS`), each platform declares the capabilities it provides
// (`Platform::provides`). A `use` of a module whose needs the active platform
// can't satisfy is a compile error at the `use` site. Modules with no row need no
// capabilities and are available everywhere (pure compute: `core.list`, `core.dict`,
// the auto-imported `core.ref`/`option`/`result`, …).

/// A host-provided capability. A module needs some set of these; a platform
/// provides some set. Grows as new host surfaces (e.g. a future `core.dom`) land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
	/// Filesystem reads/writes (`core.io` file ops).
	Fs,
	/// Standard streams — stdin/stdout/stderr (`io.read`, `io.print`).
	Stdio,
	/// Network sockets / outbound connections.
	Net,
	/// Wall-clock + monotonic clock reads (`time.now`, `time.monotonic`).
	Clock,
	/// Cryptographic / OS entropy (`random.*` seeds, `uuid` v4/v7).
	Entropy,
	/// Process/env surface — `io.args`, `io.env`, `io.exit`.
	Process,
	/// Browser DOM access (future `core.dom`).
	Dom,
	/// Browser `fetch` / outbound HTTP from the client (future).
	Fetch,
	/// Browser timers — `setTimeout`/`requestAnimationFrame` (future).
	Timer,
}

/// A deploy target = a host-capability profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Platform {
	/// The bytecode VM (`pluma run`, tests, scripts). Provides every capability,
	/// so module gating never rejects anything. The default.
	#[default]
	Native,
	/// A server/machine wasm host (filesystem, stdio, net, clock, entropy, process).
	Server,
	/// A browser wasm host (DOM, fetch, timers, console).
	Browser,
}

impl Platform {
	/// Whether this platform's host provides `cap`.
	pub fn provides(self, cap: Capability) -> bool {
		use Capability::*;
		match self {
			// Native is the full-capability dev/test profile — everything.
			Platform::Native => true,
			Platform::Server => matches!(cap, Fs | Stdio | Net | Clock | Entropy | Process),
			// `Stdio` here is `console.log` — keeps `print`/`debug` working in the browser.
			Platform::Browser => matches!(cap, Dom | Fetch | Timer | Stdio),
		}
	}

	/// A human-facing name for diagnostics ("not available on the browser target").
	pub fn label(self) -> &'static str {
		match self {
			Platform::Native => "native",
			Platform::Server => "server",
			Platform::Browser => "browser",
		}
	}

	/// The capabilities a module needs but this platform doesn't provide. Empty
	/// means the module is available here. `module_capabilities` returns `&[]` for
	/// any module without a row, so ungated modules always come back empty.
	pub fn missing_capabilities(self, module_name: &str) -> Vec<Capability> {
		module_capabilities(module_name)
			.iter()
			.copied()
			.filter(|c| !self.provides(*c))
			.collect()
	}
}

/// The capabilities a stdlib module requires. Only gated modules need a row;
/// everything else (and every user module) defaults to `&[]` — never gated.
pub fn module_capabilities(module_name: &str) -> &'static [Capability] {
	use Capability::*;
	match module_name {
		"core.io" => &[Fs, Stdio, Process],
		_ => &[],
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn native_provides_everything() {
		for cap in [
			Capability::Fs,
			Capability::Stdio,
			Capability::Net,
			Capability::Clock,
			Capability::Entropy,
			Capability::Process,
			Capability::Dom,
			Capability::Fetch,
			Capability::Timer,
		] {
			assert!(
				Platform::Native.provides(cap),
				"Native should provide {cap:?}"
			);
		}
	}

	#[test]
	fn core_io_gating_by_platform() {
		// Native + Server satisfy core.io; the browser is missing Fs + Process.
		assert!(Platform::Native.missing_capabilities("core.io").is_empty());
		assert!(Platform::Server.missing_capabilities("core.io").is_empty());
		assert_eq!(
			Platform::Browser.missing_capabilities("core.io"),
			vec![Capability::Fs, Capability::Process]
		);
	}

	#[test]
	fn ungated_modules_need_nothing() {
		// Unlisted (pure-compute) modules require no capabilities anywhere.
		for p in [Platform::Native, Platform::Server, Platform::Browser] {
			assert!(p.missing_capabilities("core.list").is_empty());
			assert!(p.missing_capabilities("some.user.module").is_empty());
		}
	}
}
