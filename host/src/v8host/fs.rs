// std/sys/io reads / fs, plus the process surface (argv/env/exit). Each callback reads
// path/data out of scratch, runs the `std::fs`/stdin op, delivers bytes back into the
// caller's `(dst, cap)` buffer (overflow → `read_stash` for `io-copyout`), and sets
// `last_error` on failure.

use super::Ctx;
use super::marshal::{argi, ctx_and_mem, deliver_read_v8, read_mem, read_str, write_mem};

pub(super) fn cb_io_read(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let line = ctx.state.read_line();
	let n = match line {
		Some(l) => deliver_read_v8(scope, mem, ctx, dst, cap, l.into_bytes()),
		None => {
			ctx.state.last_error = "EOF".to_string();
			-1
		}
	};
	rv.set_int32(n);
}

pub(super) fn cb_io_read_all(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let bytes = ctx.state.read_rest();
	let s = String::from_utf8_lossy(&bytes).into_owned();
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, s.into_bytes()));
}

pub(super) fn cb_io_read_all_bytes(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let bytes = ctx.state.read_rest();
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, bytes));
}

fn read_file_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	as_bytes: bool,
	rv: &mut v8::ReturnValue,
) {
	let (pp, pl) = (argi(scope, args, 0), argi(scope, args, 1));
	let (dst, cap) = (argi(scope, args, 2), argi(scope, args, 3));
	let (ctx, mem) = ctx_and_mem(scope, args);
	let path = read_str(scope, mem, pp, pl);
	let res = if as_bytes {
		std::fs::read(&path)
	} else {
		std::fs::read_to_string(&path).map(String::into_bytes)
	};
	let n = match res {
		Ok(b) => deliver_read_v8(scope, mem, ctx, dst, cap, b),
		Err(e) => {
			ctx.state.last_error = e.to_string();
			-1
		}
	};
	rv.set_int32(n);
}
pub(super) fn cb_read_file(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	read_file_impl(s, &a, false, &mut r);
}
pub(super) fn cb_read_file_bytes(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	read_file_impl(s, &a, true, &mut r);
}

pub(super) fn cb_read_dir(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (pp, pl) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (dst, cap) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let path = read_str(scope, mem, pp, pl);
	let n = match std::fs::read_dir(&path) {
		Ok(entries) => {
			let mut names: Vec<String> = Vec::new();
			let mut err: Option<String> = None;
			for e in entries {
				match e {
					Ok(e) => names.push(e.file_name().to_string_lossy().into_owned()),
					Err(e) => {
						err = Some(e.to_string());
						break;
					}
				}
			}
			match err {
				Some(msg) => {
					ctx.state.last_error = msg;
					-1
				}
				None => {
					names.sort();
					let mut blob = Vec::new();
					for nm in &names {
						blob.extend_from_slice(nm.as_bytes());
						blob.push(0);
					}
					deliver_read_v8(scope, mem, ctx, dst, cap, blob)
				}
			}
		}
		Err(e) => {
			ctx.state.last_error = e.to_string();
			-1
		}
	};
	rv.set_int32(n);
}

fn write_file_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	append: bool,
	rv: &mut v8::ReturnValue,
) {
	let (pp, pl) = (argi(scope, args, 0), argi(scope, args, 1));
	let (dp, dl) = (argi(scope, args, 2), argi(scope, args, 3));
	let (ctx, mem) = ctx_and_mem(scope, args);
	let path = read_str(scope, mem, pp, pl);
	let data = read_mem(scope, mem, dp.max(0) as usize, dl.max(0) as usize);
	let res = if append {
		use std::io::Write;
		std::fs::OpenOptions::new()
			.create(true)
			.append(true)
			.open(&path)
			.and_then(|mut f| f.write_all(&data))
	} else {
		std::fs::write(&path, &data)
	};
	rv.set_int32(io_status(ctx, res));
}
pub(super) fn cb_write_file(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	write_file_impl(s, &a, false, &mut r);
}
pub(super) fn cb_append_file(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	write_file_impl(s, &a, true, &mut r);
}

fn path_op_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	op: impl FnOnce(&str) -> std::io::Result<()>,
	rv: &mut v8::ReturnValue,
) {
	let (pp, pl) = (argi(scope, args, 0), argi(scope, args, 1));
	let (ctx, mem) = ctx_and_mem(scope, args);
	let path = read_str(scope, mem, pp, pl);
	let res = op(&path);
	rv.set_int32(io_status(ctx, res));
}
pub(super) fn cb_delete_file(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	path_op_impl(s, &a, |p| std::fs::remove_file(p), &mut r);
}
pub(super) fn cb_make_dir(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	path_op_impl(s, &a, |p| std::fs::create_dir_all(p), &mut r);
}

fn query_impl(
	scope: &mut v8::HandleScope,
	args: &v8::FunctionCallbackArguments,
	is_dir: bool,
	rv: &mut v8::ReturnValue,
) {
	let (pp, pl) = (argi(scope, args, 0), argi(scope, args, 1));
	let (_ctx, mem) = ctx_and_mem(scope, args);
	let path = read_str(scope, mem, pp, pl);
	let p = std::path::Path::new(&path);
	let b = if is_dir { p.is_dir() } else { p.exists() };
	rv.set_int32(b as i32);
}
pub(super) fn cb_file_exists(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	query_impl(s, &a, false, &mut r);
}
pub(super) fn cb_is_dir(
	s: &mut v8::HandleScope,
	a: v8::FunctionCallbackArguments,
	mut r: v8::ReturnValue,
) {
	query_impl(s, &a, true, &mut r);
}

/// `io-last-error(dst, cap) -> len`: write the stashed message into scratch (truncated
/// to `cap`); errno strings are short, so no overflow stash.
pub(super) fn cb_last_error(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let msg = ctx.state.last_error.clone();
	let bytes = msg.as_bytes();
	let len = bytes.len().min(cap.max(0) as usize);
	write_mem(scope, mem, dst.max(0) as usize, &bytes[..len]);
	rv.set_int32(len as i32);
}

/// `io-copyout(dst)`: drain the read stash into scratch at `dst` (the overflow path).
pub(super) fn cb_io_copyout(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	let dst = argi(scope, &args, 0);
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let stash = std::mem::take(&mut ctx.state.read_stash);
	write_mem(scope, mem, dst.max(0) as usize, &stash);
}

/// `io-args(dst, cap) -> len`: deliver the program's argv as a NUL-terminated name
/// blob in scratch (each arg followed by a `\0`, the `__read_names` shape), exactly
/// like `io-read-dir`. The wasm side splits it into a bare `$list` of `$str` (no
/// `result` wrapper — argv never fails). An empty argv writes nothing (len 0).
pub(super) fn cb_io_args(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let mut blob = Vec::new();
	for a in &ctx.state.args {
		blob.extend_from_slice(a.as_bytes());
		blob.push(0);
	}
	rv.set_int32(deliver_read_v8(scope, mem, ctx, dst, cap, blob));
}

/// `io-env(name_ptr, name_len, dst, cap) -> len`: look up an environment variable.
/// Deliver the value bytes to `(dst, cap)` and return its length on a hit (`some
/// value`), or return `-1` for an unset var (`none`) — the wasm side shapes the `len`
/// into an `option string`.
pub(super) fn cb_io_env(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (np, nl) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (dst, cap) = (argi(scope, &args, 2), argi(scope, &args, 3));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let name = read_str(scope, mem, np, nl);
	// Reserved name: a parallel `pluma test` shard reads its `(id, count)` here
	// rather than from a real env var or argv, so test code that inspects the real
	// environment is unaffected. The synthesized test entry is the only caller.
	let value = if name == "PLUMA_TEST_SHARD" {
		ctx.state.shard.map(|(id, count)| format!("{id} {count}"))
	} else {
		std::env::var(&name).ok()
	};
	let n = match value {
		Some(v) => deliver_read_v8(scope, mem, ctx, dst, cap, v.into_bytes()),
		None => -1, // unset (or non-UTF-8) -> `none`.
	};
	rv.set_int32(n);
}

/// `io-cwd(dst, cap) -> len`: the process's current working directory as a string (the
/// `(dst, cap)` read shape, like `io-read`; `len < 0` → `err` with the OS message). Backs
/// `process.cwd` and `path.project-root`.
pub(super) fn cb_io_cwd(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	mut rv: v8::ReturnValue,
) {
	let (dst, cap) = (argi(scope, &args, 0), argi(scope, &args, 1));
	let (ctx, mem) = ctx_and_mem(scope, &args);
	let n = match std::env::current_dir() {
		Ok(p) => deliver_read_v8(
			scope,
			mem,
			ctx,
			dst,
			cap,
			p.to_string_lossy().into_owned().into_bytes(),
		),
		Err(e) => {
			ctx.state.last_error = e.to_string();
			-1
		}
	};
	rv.set_int32(n);
}

/// `io-exit(code)`: stop the program immediately with `code` via
/// `std::process::exit` (`io.exit` diverges). Streamed stdout/stderr are already
/// flushed per-write, so no draining is needed. Never returns.
pub(super) fn cb_io_exit(
	scope: &mut v8::HandleScope,
	args: v8::FunctionCallbackArguments,
	_r: v8::ReturnValue,
) {
	std::process::exit(argi(scope, &args, 0));
}

/// Shape an fs `Result<()>` into a `(0 ok / 2 err)` status, stashing the errno text.
fn io_status(ctx: &mut Ctx, res: std::io::Result<()>) -> i32 {
	match res {
		Ok(()) => 0,
		Err(e) => {
			ctx.state.last_error = e.to_string();
			2
		}
	}
}
