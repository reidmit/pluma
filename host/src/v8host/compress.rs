// Native import callbacks for `std/compress` (`crate::compress`). Each is a byte
// transform that rides the same marshalling ABI as `io.read-file-bytes` (classified
// `IoKind::ReadFileBytes` on the wasm side): source bytes in scratch at `(src, len)`, the
// result delivered into `(dst, cap)` (overflow stashed for `io-copyout`), returning the
// true length — or `-1` with the message in `last_error` when decoding malformed input,
// which `__io_result` shapes into `result.err`. Encoding never fails.

use super::marshal::{argi, ctx_and_mem, deliver_read_v8, read_mem};

/// Run one codec over the scratch `(src, len)` bytes and deliver the result, shared by the
/// four callbacks below.
fn run(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	rv: &mut v8::ReturnValue,
	codec: fn(&[u8]) -> Result<Vec<u8>, String>,
) {
	let (sp, sl) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (dst, cap) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let src = read_mem(scope, mem, sp.max(0) as usize, sl.max(0) as usize);
	let n = match codec(&src) {
		Ok(out) => deliver_read_v8(scope, mem, ctx, dst, cap, out),
		Err(e) => {
			ctx.state.last_error = e;
			-1
		}
	};
	rv.set_int32(n);
}

pub(super) fn cb_gzip_encode(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	run(scope, args, &mut rv, crate::compress::gzip_encode);
}

pub(super) fn cb_gzip_decode(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	run(scope, args, &mut rv, crate::compress::gzip_decode);
}

pub(super) fn cb_brotli_encode(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	run(scope, args, &mut rv, crate::compress::brotli_encode);
}

pub(super) fn cb_brotli_decode(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	run(scope, args, &mut rv, crate::compress::brotli_decode);
}
