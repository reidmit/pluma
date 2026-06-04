use formatter::format_source;

fn fmt(src: &str) -> String {
	format_source(src.as_bytes()).expect("parse should succeed")
}

#[test]
fn simple_def() {
	let out = fmt("def x = 1\n");
	assert_eq!(out, "def x = 1\n");
}

#[test]
fn idempotent_simple() {
	let src = "def x = 1\n";
	let once = fmt(src);
	let twice = fmt(&once);
	assert_eq!(once, twice);
}

#[test]
fn imports_before_defs() {
	let out = fmt("use std.list\n\ndef x = 1\n");
	assert_eq!(out, "use std.list\n\ndef x = 1\n");
}

#[test]
fn top_level_fun_always_multi_line() {
	// Top-level `def NAME = fun { ... }` always breaks the body, even
	// when it would fit on one line.
	let out = fmt("def f = fun x { x + 1 }\n");
	assert_eq!(out, "def f = fun x {\n\tx + 1\n}\n");
}

#[test]
fn fun_multi_expr_breaks() {
	let out = fmt("def main = fun {\n  print 1\n  print 2\n}\n");
	assert_eq!(out, "def main = fun {\n\tprint 1\n\tprint 2\n}\n");
}

#[test]
fn list_short_inline() {
	let out = fmt("def xs = [1, 2, 3]\n");
	assert_eq!(out, "def xs = [1, 2, 3]\n");
}

#[test]
fn enum_block() {
	let out = fmt("enum color { red\n  green\n  blue\n}\n");
	assert_eq!(out, "enum color {\n\tred\n\tgreen\n\tblue\n}\n");
}

#[test]
fn leading_comments_preserved() {
	let src = "# hello\ndef x = 1\n";
	let out = fmt(src);
	assert_eq!(out, "# hello\ndef x = 1\n");
}
