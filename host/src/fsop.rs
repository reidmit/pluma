// Engine-independent filesystem ops for `std/sys/fs`. One `dispatch` over an op-code
// covers the whole surface; both the async path (a pool worker — host/src/offload.rs) and the
// synchronous `-sync` path run the same code, so behaviour can't drift between them. The
// V8 glue (reading args out of scratch, delivering bytes back) lives in `v8host`; nothing
// here touches V8. Results come back as an `OpResult` the callers shape into the `result`
// ABI (`Bytes` ok payload / `Err` message; `Nothing` for the void ops).
//
// Op-codes are mirrored by the private `op-*` constants in `std/sys/fs.pa`; keep them in
// sync. The Pluma wrappers interpret the `Bytes` payload per op (decode text, split the
// dir blob, parse the stat record, read the "1"/"0" of a query).

use crate::offload::OpResult;

pub(crate) mod op {
	pub(crate) const READ_TEXT: i32 = 0;
	pub(crate) const READ_BYTES: i32 = 1;
	pub(crate) const READ_DIR: i32 = 2;
	pub(crate) const STAT: i32 = 3;
	pub(crate) const WRITE: i32 = 4;
	pub(crate) const APPEND: i32 = 5;
	pub(crate) const MAKE_DIR: i32 = 6;
	pub(crate) const REMOVE: i32 = 7;
	pub(crate) const REMOVE_ALL: i32 = 8;
	pub(crate) const RENAME: i32 = 9;
	pub(crate) const COPY: i32 = 10;
	pub(crate) const EXISTS: i32 = 11;
	pub(crate) const IS_FILE: i32 = 12;
	pub(crate) const IS_DIR: i32 = 13;
}

/// Run one fs op. `path` is the primary path; `data` is the write payload (write/append)
/// or the destination path as UTF-8 bytes (rename/copy), ignored otherwise.
pub(crate) fn dispatch(op: i32, path: &str, data: &[u8]) -> OpResult {
	match op {
		op::READ_TEXT => match std::fs::read_to_string(path) {
			Ok(s) => OpResult::Bytes(s.into_bytes()),
			Err(e) => OpResult::Err(e.to_string()),
		},
		op::READ_BYTES => match std::fs::read(path) {
			Ok(b) => OpResult::Bytes(b),
			Err(e) => OpResult::Err(e.to_string()),
		},
		op::READ_DIR => read_dir(path),
		op::STAT => stat(path),
		op::WRITE => void(std::fs::write(path, data)),
		op::APPEND => append(path, data),
		op::MAKE_DIR => void(std::fs::create_dir_all(path)),
		op::REMOVE => remove(path, false),
		op::REMOVE_ALL => remove(path, true),
		op::RENAME => void(std::fs::rename(path, dest(data))),
		op::COPY => void(std::fs::copy(path, dest(data)).map(|_| ())),
		op::EXISTS => boolean(std::path::Path::new(path).exists()),
		op::IS_FILE => boolean(
			std::fs::metadata(path)
				.map(|m| m.is_file())
				.unwrap_or(false),
		),
		op::IS_DIR => boolean(std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)),
		_ => OpResult::Err(format!("fs: unknown op {op}")),
	}
}

/// The destination path of a rename/copy (the `data` arg as UTF-8).
fn dest(data: &[u8]) -> String {
	String::from_utf8_lossy(data).into_owned()
}

/// Shape a `()`-returning `io::Result` into `Nothing`/`Err`.
fn void(res: std::io::Result<()>) -> OpResult {
	match res {
		Ok(()) => OpResult::Nothing,
		Err(e) => OpResult::Err(e.to_string()),
	}
}

/// A `bool` query (`exists`/`is-file`/`is-dir`): infallible, an `"1"`/`"0"` byte the
/// Pluma wrapper reads back into a `bool`.
fn boolean(b: bool) -> OpResult {
	OpResult::Bytes(vec![if b { b'1' } else { b'0' }])
}

fn append(path: &str, data: &[u8]) -> OpResult {
	use std::io::Write;
	let res = std::fs::OpenOptions::new()
		.create(true)
		.append(true)
		.open(path)
		.and_then(|mut f| f.write_all(data));
	void(res)
}

/// Remove a file or directory. `all` → recursive (`rm -rf`); otherwise a file or an
/// *empty* directory (a non-empty dir errors, matching `rmdir`).
fn remove(path: &str, all: bool) -> OpResult {
	let md = match std::fs::symlink_metadata(path) {
		Ok(m) => m,
		Err(e) => return OpResult::Err(e.to_string()),
	};
	let res = if md.is_dir() {
		if all {
			std::fs::remove_dir_all(path)
		} else {
			std::fs::remove_dir(path)
		}
	} else {
		std::fs::remove_file(path)
	};
	void(res)
}

/// `read-dir`: the entry names (not full paths), sorted, NUL-separated with a trailing
/// NUL after each — the same blob the wasm `__read_names` helper splits into a `$list`.
fn read_dir(path: &str) -> OpResult {
	let entries = match std::fs::read_dir(path) {
		Ok(e) => e,
		Err(e) => return OpResult::Err(e.to_string()),
	};
	let mut names: Vec<String> = Vec::new();
	for e in entries {
		match e {
			Ok(e) => names.push(e.file_name().to_string_lossy().into_owned()),
			Err(e) => return OpResult::Err(e.to_string()),
		}
	}
	names.sort();
	let mut blob = Vec::new();
	for nm in &names {
		blob.extend_from_slice(nm.as_bytes());
		blob.push(0);
	}
	OpResult::Bytes(blob)
}

/// `stat`: `"<kind>\t<size>\t<modified-nanos>"` (the Pluma wrapper parses it into a
/// `file-info` record). `kind` is the entry's own type via `symlink_metadata`, so a
/// symlink reports `symlink` rather than its target. `modified-nanos` is 0 when the OS
/// doesn't report an mtime.
fn stat(path: &str) -> OpResult {
	let m = match std::fs::symlink_metadata(path) {
		Ok(m) => m,
		Err(e) => return OpResult::Err(e.to_string()),
	};
	let kind = if m.is_dir() {
		"dir"
	} else if m.is_file() {
		"file"
	} else if m.file_type().is_symlink() {
		"symlink"
	} else {
		"other"
	};
	let nanos = m
		.modified()
		.ok()
		.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
		.map(|d| d.as_nanos() as i64)
		.unwrap_or(0);
	OpResult::Bytes(format!("{kind}\t{}\t{nanos}", m.len()).into_bytes())
}
