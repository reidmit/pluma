// Baked-in stdlib `.pa` sources. Each entry is `(module-name,
// source-text)`. Loaded via `include_str!` so the language doesn't
// depend on an external stdlib directory — `cargo build` is enough.
//
// New modules are added as `.pa` files here, each def linked to a Rust
// implementation via a `built-in "tag"` expression.

pub fn stdlib_sources() -> &'static [(&'static str, &'static str)] {
	&[
		("core.regex", include_str!("stdlib/regex.pa")),
		("core.list", include_str!("stdlib/list.pa")),
		("core.dict", include_str!("stdlib/dict.pa")),
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
		("core.hex", include_str!("stdlib/hex.pa")),
		("core.base64", include_str!("stdlib/base64.pa")),
		("core.random", include_str!("stdlib/random.pa")),
		("core.uuid", include_str!("stdlib/uuid.pa")),
		("core.time", include_str!("stdlib/time.pa")),
	]
}

pub fn lookup_stdlib_source(module_name: &str) -> Option<&'static str> {
	stdlib_sources()
		.iter()
		.find(|(name, _)| *name == module_name)
		.map(|(_, source)| *source)
}
