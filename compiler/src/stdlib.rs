// Baked-in stdlib `.pa` sources. Each entry is `(module-name,
// source-text)`. Loaded via `include_str!` so the language doesn't
// depend on an external stdlib directory — `cargo build` is enough.
//
// New modules are migrated from `vm/src/stdlib.rs`'s `native_modules`
// over time; see STDLIB.md for the design and Phase 4 sequencing.

pub fn stdlib_sources() -> &'static [(&'static str, &'static str)] {
	&[
		("core.regex", include_str!("stdlib/regex.pa")),
		("core.list", include_str!("stdlib/list.pa")),
		("core.string", include_str!("stdlib/string.pa")),
		("core.math", include_str!("stdlib/math.pa")),
		("core.bytes", include_str!("stdlib/bytes.pa")),
		("core.io", include_str!("stdlib/io.pa")),
		("core.ref", include_str!("stdlib/ref.pa")),
		("core.option", include_str!("stdlib/option.pa")),
		("core.result", include_str!("stdlib/result.pa")),
		("core.json", include_str!("stdlib/json.pa")),
		("core.assert", include_str!("stdlib/assert.pa")),
		("core.package", include_str!("stdlib/package.pa")),
	]
}

pub fn lookup_stdlib_source(module_name: &str) -> Option<&'static str> {
	stdlib_sources()
		.iter()
		.find(|(name, _)| *name == module_name)
		.map(|(_, source)| *source)
}
