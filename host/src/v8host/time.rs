// std.time (the `Clock` capability). Wall clock + monotonic clock + blocking sleep +
// strtime parse, using the `jiff` crate. `time.now` is an `instant` (unix nanos),
// `time.monotonic` a `duration` (nanos since a process-start anchor); both cross as i64
// BigInts and the wasm side boxes them under the right tag.

use super::marshal::{argi, ctx_and_mem, read_str, write_mem};

/// Process-start anchor for `time.monotonic` (a static `OnceLock`).
static MONOTONIC_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// `time-now() -> i64`: wall-clock unix nanos (boxed `instant` in wasm).
pub(super) fn cb_time_now(
	scope: &mut v8::HandleScope,
	_a: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let n = jiff::Timestamp::now().as_nanosecond() as i64;
	rv.set(v8::BigInt::new_from_i64(scope, n).into());
}

/// `time-monotonic() -> i64`: nanos since the process-start anchor (boxed `duration`).
pub(super) fn cb_time_monotonic(
	scope: &mut v8::HandleScope,
	_a: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let start = MONOTONIC_START.get_or_init(std::time::Instant::now);
	let n = start.elapsed().as_nanos() as i64;
	rv.set(v8::BigInt::new_from_i64(scope, n).into());
}

/// `time-sleep(i64 nanos)`: block the thread (synchronous host call, like `net-poll`).
/// Returns nothing.
pub(super) fn cb_time_sleep(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	let nanos = args
		.get(0)
		.to_big_int(scope)
		.map(|b| b.i64_value().0)
		.unwrap_or(0);
	if nanos > 0 {
		std::thread::sleep(std::time::Duration::from_nanos(nanos as u64));
	}
}

/// `time-parse(fp, fl, ip, il, dst) -> status`: strtime-parse `input` per `fmt`. On
/// success write the i64 nanos (LE) to scratch at `dst` and return 0; on failure stash
/// the message for `io-last-error` and return 1 — the wasm side shapes `__io_result`
/// (`ok (instant nanos)` / `err message`).
pub(super) fn cb_time_parse(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (fp, fl) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ip, il) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let dst = argi(scope, &args, 4);
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let fmt = read_str(scope, mem, fp, fl);
	let input = read_str(scope, mem, ip, il);
	let status = match parse_with_format(&fmt, &input) {
		Ok(nanos) => {
			write_mem(scope, mem, dst.max(0) as usize, &nanos.to_le_bytes());
			0
		}
		Err(e) => {
			ctx.state.last_error = e;
			1
		}
	};
	rv.set_int32(status);
}

/// `time.parse`: strtime-parse, then prefer a complete timestamp, else interpret a
/// bare civil datetime as UTC.
fn parse_with_format(fmt: &str, input: &str) -> Result<i64, String> {
	let tm = jiff::fmt::strtime::parse(fmt, input).map_err(|e| format!("time: {}", e))?;
	if let Ok(ts) = tm.to_timestamp() {
		return Ok(ts.as_nanosecond() as i64);
	}
	let dt = tm
		.to_datetime()
		.map_err(|e| format!("time: incomplete date/time: {}", e))?;
	dt.to_zoned(jiff::tz::TimeZone::UTC)
		.map(|z| z.timestamp().as_nanosecond() as i64)
		.map_err(|e| format!("time: {}", e))
}
