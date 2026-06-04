// Baked-in stdlib `.pa` sources. Each entry is `(module-name,
// source-text)`. Loaded via `include_str!` so the language doesn't
// depend on an external stdlib directory — `cargo build` is enough.
//
// New modules are added as `.pa` files here, each def linked to a Rust
// implementation via a `built-in "tag"` expression.

pub fn stdlib_sources() -> &'static [(&'static str, &'static str)] {
	&[
		("std.regex", include_str!("stdlib/regex.pa")),
		("std.list", include_str!("stdlib/list.pa")),
		("std.dict", include_str!("stdlib/dict.pa")),
		("std.string", include_str!("stdlib/string.pa")),
		("std.math", include_str!("stdlib/math.pa")),
		("std.bytes", include_str!("stdlib/bytes.pa")),
		("std.sys.io", include_str!("stdlib/sys/io.pa")),
		("std.ref", include_str!("stdlib/ref.pa")),
		("std.option", include_str!("stdlib/option.pa")),
		("std.result", include_str!("stdlib/result.pa")),
		("std.json", include_str!("stdlib/json.pa")),
		("std.assert", include_str!("stdlib/assert.pa")),
		("std.test", include_str!("stdlib/test.pa")),
		("std.sys.terminal", include_str!("stdlib/sys/terminal.pa")),
		("std.package", include_str!("stdlib/package.pa")),
		("std.hex", include_str!("stdlib/hex.pa")),
		("std.base64", include_str!("stdlib/base64.pa")),
		("std.random", include_str!("stdlib/random.pa")),
		("std.uuid", include_str!("stdlib/uuid.pa")),
		("std.time", include_str!("stdlib/time.pa")),
		("std.task", include_str!("stdlib/task.pa")),
		("std.sys.net", include_str!("stdlib/sys/net.pa")),
		("std.sys.http", include_str!("stdlib/sys/http.pa")),
		("std.web.dom", include_str!("stdlib/web/dom.pa")),
		("std.web.html", include_str!("stdlib/web/html.pa")),
		("std.web.events", include_str!("stdlib/web/events.pa")),
		("std.web.app", include_str!("stdlib/web/app.pa")),
	]
}

pub fn lookup_stdlib_source(module_name: &str) -> Option<&'static str> {
	stdlib_sources()
		.iter()
		.find(|(name, _)| *name == module_name)
		.map(|(_, source)| *source)
}
