// Native import callback for SHA-1 (`crate::crypto`). It rides the same marshalling ABI as
// `io.read-file-bytes` (classified `IoKind::ReadFileBytes` on the wasm side): source bytes
// in scratch at `(src, len)`, the 20-byte digest delivered into `(dst, cap)` (overflow
// stashed for `io-copyout`), returning the true length. Hashing never fails.

use super::marshal::{argi, ctx_and_mem, deliver_read_v8, read_mem};

pub(super) fn cb_sha1(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (sp, sl) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (dst, cap) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let src = read_mem(scope, mem, sp.max(0) as usize, sl.max(0) as usize);
	let n = match crate::crypto::sha1(&src) {
		Ok(out) => deliver_read_v8(scope, mem, ctx, dst, cap, out),
		Err(e) => {
			ctx.state.last_error = e;
			-1
		}
	};
	rv.set_int32(n);
}
