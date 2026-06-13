// std/regex (the matcher). Pluma's `regex` backtick literal lowers to a `regex-pattern`
// tree, which `std/regex` translates to an equivalent JS `RegExp` source string; this
// callback runs that pattern over the subject with V8's own Irregexp engine — the same
// regex engine Node and Deno use — and returns the match spans.
//
// Both strings cross as latin1 (one wasm byte = one UTF-16 code unit), so V8's code-unit
// offsets coincide exactly with the byte offsets Pluma's `match {start, end}` contract
// promises, even for non-ASCII input. The result is a packed little-endian i32 buffer —
// per match `[match_start, match_end, g1_start, g1_end, …, gN_start, gN_end]`, with
// `-1,-1` for a capture group that didn't participate — which `std/regex` unpacks into
// `match` records (it owns the group names, so only offsets travel back).

use super::marshal::{argi, ctx_and_mem, deliver_read_v8, read_mem};

// Scan the subject for every non-overlapping match and flatten the offsets into an
// `Int32Array`. The `d` (`hasIndices`) flag exposes per-group offsets via `m.indices`;
// the `g` flag drives the left-to-right scan. Capturing groups are all positional (the
// translator emits `(…)` for `p-capture` and `(?:…)` for grouping), so `m.length - 1`
// equals Pluma's capture count and the indices line up. An empty match advances by one
// code unit, matching the pure-Pluma matcher's `collect-spans`.
const SCAN_JS: &str = r#"(function (src, subject) {
	const re = new RegExp(src, 'gd');
	const out = [];
	let m;
	while ((m = re.exec(subject)) !== null) {
		out.push(m.index, m.index + m[0].length);
		const ind = m.indices;
		for (let g = 1; g < m.length; g++) {
			const gi = ind[g];
			if (gi) { out.push(gi[0], gi[1]); } else { out.push(-1, -1); }
		}
		if (m[0].length === 0) { re.lastIndex++; }
	}
	return Int32Array.from(out);
})"#;

pub(super) fn cb_regex_find_all(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (pp, pl, ip, il) = (
		argi(scope, &args, 0),
		argi(scope, &args, 1),
		argi(scope, &args, 2),
		argi(scope, &args, 3),
	);
	let (dst, cap) = (argi(scope, &args, 4), argi(scope, &args, 5));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let pat_bytes = read_mem(scope, mem, pp.max(0) as usize, pl.max(0) as usize);
	let input_bytes = read_mem(scope, mem, ip.max(0) as usize, il.max(0) as usize);

	// Build the latin1 pattern + subject, compile the scan helper, and run it. On any V8
	// exception (an invalid translated pattern — a compiler bug, not a user error) record
	// the message and return zero matches rather than aborting the host.
	let bytes = match scan_offsets(scope, &pat_bytes, &input_bytes) {
		Some(b) => b,
		None => {
			ctx.state.last_error = "regex match failed".to_string();
			Vec::new()
		}
	};
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, bytes));
}

/// Run `SCAN_JS` over the (latin1) pattern + subject and return the `Int32Array`'s raw
/// little-endian bytes. `None` on any compile/run exception or unexpected return shape.
fn scan_offsets(scope: &mut v8::HandleScope, pat: &[u8], input: &[u8]) -> Option<Vec<u8>> {
	let pat_str = v8::String::new_from_one_byte(scope, pat, v8::NewStringType::Normal)?;
	let subject = v8::String::new_from_one_byte(scope, input, v8::NewStringType::Normal)?;
	let source = v8::String::new(scope, SCAN_JS)?;
	let script = v8::Script::compile(scope, source, None)?;
	let func = v8::Local::<v8::Function>::try_from(script.run(scope)?).ok()?;
	let recv = v8::undefined(scope).into();
	let result = func.call(scope, recv, &[pat_str.into(), subject.into()])?;
	let view = v8::Local::<v8::ArrayBufferView>::try_from(result).ok()?;
	let mut buf = vec![0u8; view.byte_length()];
	view.copy_contents(&mut buf);
	Some(buf)
}
