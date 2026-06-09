// Shared V8 ↔ scratch-memory marshalling helpers used by every import callback: read
// the `Ctx` + exported memory out of a callback's `External` data, move bytes in and out
// of the module's scratch `"memory"`, and install a native import. Everything the
// engine-neutral ABI boundary needs that's still V8-specific lives here, so the per-
// capability callback modules stay focused on their op.

use super::Ctx;

/// `obj.<key>` as a `Local<Value>`.
pub(super) fn get_prop<'s>(
	scope: &mut v8::HandleScope<'s>,
	obj: v8::Local<v8::Object>,
	key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
	let k = v8::String::new(scope, key)?;
	obj.get(scope, k.into())
}

/// The exported memory's current backing-store data pointer + length (re-read each
/// time: `memory.grow` swaps the `ArrayBuffer`).
fn mem_slice<'s>(
	scope: &mut v8::HandleScope<'s>,
	memory: v8::Local<v8::Object>,
) -> (*mut u8, usize) {
	let buffer: v8::Local<v8::ArrayBuffer> = get_prop(scope, memory, "buffer")
		.and_then(|v| v.try_into().ok())
		.expect("memory.buffer");
	let store = buffer.get_backing_store();
	let len = store.byte_length();
	let ptr = match store.data() {
		Some(p) => p.as_ptr() as *mut u8,
		None => std::ptr::null_mut(),
	};
	(ptr, len)
}

/// Read `len` bytes of the wasm memory at `off`.
pub(super) fn read_mem(
	scope: &mut v8::HandleScope,
	memory: v8::Local<v8::Object>,
	off: usize,
	len: usize,
) -> Vec<u8> {
	let (ptr, cap) = mem_slice(scope, memory);
	if ptr.is_null() || off + len > cap {
		return Vec::new();
	}
	unsafe { std::slice::from_raw_parts(ptr.add(off), len).to_vec() }
}

/// Write `data` into the wasm memory at `off`.
pub(super) fn write_mem(
	scope: &mut v8::HandleScope,
	memory: v8::Local<v8::Object>,
	off: usize,
	data: &[u8],
) {
	let (ptr, cap) = mem_slice(scope, memory);
	if ptr.is_null() || off + data.len() > cap {
		return;
	}
	unsafe { std::slice::from_raw_parts_mut(ptr.add(off), data.len()).copy_from_slice(data) }
}

/// Recover the `Ctx` from a callback's `External` data, plus a `Local` of the exported
/// memory opened in the callback's scope.
pub(super) fn ctx_and_mem<'s>(
	scope: &mut v8::HandleScope<'s>,
	args: &v8::FunctionCallbackArguments,
) -> (&'s mut Ctx, v8::Local<'s, v8::Object>) {
	let ext = v8::Local::<v8::External>::try_from(args.data()).expect("callback External data");
	let ctx = unsafe { &mut *(ext.value() as *mut Ctx) };
	let mem = ctx.memory.as_ref().expect("memory set before callbacks");
	let mem = v8::Local::new(scope, mem);
	(ctx, mem)
}

/// An `i32` callback argument.
pub(super) fn argi(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	i: i32,
) -> i32 {
	args.get(i).int32_value(scope).unwrap_or(0)
}

/// Install one `pluma.<name>` native import on `pluma`, wired to the shared `Ctx` via
/// `data`. Generic over the callback because `MapFnTo` requires the zero-sized fn item
/// (a fn pointer would have nonzero size and fail its const check).
pub(super) fn register<'s>(
	scope: &mut v8::HandleScope<'s>,
	pluma: v8::Local<'s, v8::Object>,
	data: v8::Local<'s, v8::Value>,
	name: &str,
	cb: impl v8::MapFnTo<v8::FunctionCallback>,
) {
	let key = v8::String::new(scope, name).unwrap();
	let f = v8::Function::builder(cb).data(data).build(scope).unwrap();
	pluma.set(scope, key.into(), f.into());
}

/// Set a multi-result import return as a `[a, b]` JS array (how V8 surfaces a multi-value
/// wasm import result). Used by the `(status, n)`-returning net + offload ops.
pub(super) fn set_pair(scope: &mut v8::HandleScope, rv: &mut v8::ReturnValue, a: i32, b: i32) {
	let arr = v8::Array::new(scope, 2);
	let av: v8::Local<v8::Value> = v8::Integer::new(scope, a).into();
	arr.set_index(scope, 0, av);
	let bv: v8::Local<v8::Value> = v8::Integer::new(scope, b).into();
	arr.set_index(scope, 1, bv);
	rv.set(arr.into());
}

/// A UTF-8-lossy string read of `(ptr, len)` scratch bytes.
pub(super) fn read_str(
	scope: &mut v8::HandleScope,
	mem: v8::Local<v8::Object>,
	ptr: i32,
	len: i32,
) -> String {
	let b = read_mem(scope, mem, ptr.max(0) as usize, len.max(0) as usize);
	String::from_utf8_lossy(&b).into_owned()
}

/// Deliver a read's `bytes` to the caller's `(dst, cap)` buffer (V8 analogue of
/// `deliver_read`): write into scratch if they fit, else stash for `io-copyout`;
/// return the true length.
pub(super) fn deliver_read_v8(
	scope: &mut v8::HandleScope,
	mem: v8::Local<v8::Object>,
	ctx: &mut Ctx,
	dst: i32,
	cap: i32,
	bytes: Vec<u8>,
) -> i32 {
	let len = bytes.len();
	if len <= cap.max(0) as usize {
		write_mem(scope, mem, dst.max(0) as usize, &bytes);
	} else {
		ctx.state.read_stash = bytes;
	}
	len as i32
}
