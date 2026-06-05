// Baked-in stdlib `.pa` sources. Each entry is `(module-name,
// source-text)`. Loaded via `include_str!` so the language doesn't
// depend on an external stdlib directory — `cargo build` is enough.
//
// New modules are added as `.pa` files in the top-level `std/` directory,
// each def linked to a Rust
// implementation via a `built-in "tag"` expression.

pub fn stdlib_sources() -> &'static [(&'static str, &'static str)] {
	&[
		("std.assert", include_str!("../../std/assert.pa")),
		("std.base64", include_str!("../../std/base64.pa")),
		("std.bytes", include_str!("../../std/bytes.pa")),
		("std.dict", include_str!("../../std/dict.pa")),
		("std.hex", include_str!("../../std/hex.pa")),
		("std.json", include_str!("../../std/json.pa")),
		("std.list", include_str!("../../std/list.pa")),
		("std.local", include_str!("../../std/local.pa")),
		("std.math", include_str!("../../std/math.pa")),
		("std.option", include_str!("../../std/option.pa")),
		("std.package", include_str!("../../std/package.pa")),
		("std.random", include_str!("../../std/random.pa")),
		("std.ref", include_str!("../../std/ref.pa")),
		("std.regex", include_str!("../../std/regex.pa")),
		("std.request", include_str!("../../std/request.pa")),
		("std.result", include_str!("../../std/result.pa")),
		("std.router", include_str!("../../std/router.pa")),
		("std.stream", include_str!("../../std/stream.pa")),
		("std.string", include_str!("../../std/string.pa")),
		("std.task", include_str!("../../std/task.pa")),
		("std.test", include_str!("../../std/test.pa")),
		("std.time", include_str!("../../std/time.pa")),
		("std.uuid", include_str!("../../std/uuid.pa")),
		("std.sys.fs", include_str!("../../std/sys/fs.pa")),
		("std.sys.http", include_str!("../../std/sys/http.pa")),
		("std.sys.io", include_str!("../../std/sys/io.pa")),
		("std.sys.net", include_str!("../../std/sys/net.pa")),
		("std.sys.process", include_str!("../../std/sys/process.pa")),
		(
			"std.sys.terminal",
			include_str!("../../std/sys/terminal.pa"),
		),
		("std.web.app", include_str!("../../std/web/app.pa")),
		("std.web.dom", include_str!("../../std/web/dom.pa")),
		("std.web.events", include_str!("../../std/web/events.pa")),
		("std.web.fetch", include_str!("../../std/web/fetch.pa")),
		("std.web.html", include_str!("../../std/web/html.pa")),
	]
}

pub fn lookup_stdlib_source(module_name: &str) -> Option<&'static str> {
	stdlib_sources()
		.iter()
		.find(|(name, _)| *name == module_name)
		.map(|(_, source)| *source)
}
