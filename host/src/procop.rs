// Subprocess execution for `std/sys/process`. Rides the generic blocking-pool op channel
// (the `fs-op`/`fs-op-sync` builtins): the offload callbacks route op-codes >= `op::RUN`
// here instead of to `fsop`, so spawning a child reuses the whole marshalling/CPS/task
// machinery the async-fs surface already has. A pool worker runs the blocking
// `std::process::Command` off the scheduler thread; the result (exit status + captured
// stdout/stderr) comes back as a length-prefixed `Bytes` blob the Pluma `process` wrappers
// decode. Engine-independent — nothing here touches V8.
//
// Op-codes are mirrored by the private `op-*` constants in `std/sys/process.pa`; keep them
// in sync. The request blob (the `data` arg) is built by `encode-req` there; the response
// blob is read back by `decode-output`/`decode-status`.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::offload::OpResult;

pub(crate) mod op {
	/// Run `cmd` with args, wait, capture stdout+stderr.
	pub(crate) const RUN: i32 = 100;
	/// Run `cmd` with args, wait, but inherit the terminal's stdio (no capture).
	pub(crate) const EXEC: i32 = 101;
	/// Run `cmd` as a shell command line via `/bin/sh -c`, capture output.
	pub(crate) const SHELL: i32 = 102;
	/// Locate an executable named `cmd` on `PATH`.
	pub(crate) const WHICH: i32 = 103;
	/// Snapshot the whole inherited environment (the `cmd`/blob args are ignored).
	pub(crate) const ENV_ALL: i32 = 104;
	/// This process's own PID (the `cmd`/blob args are ignored).
	pub(crate) const PID: i32 = 105;
}

/// Whether an op-code addresses the process surface rather than `fsop`. The offload
/// callbacks use this to pick which dispatch runs on the worker.
pub(crate) fn is_proc_op(op: i32) -> bool {
	op >= op::RUN
}

/// Run one process op. `cmd` is the program (the `path` slot); `data` is the request blob
/// (cwd / stdin / args / env, see `parse_req`), ignored by `which`/`pid`.
pub(crate) fn dispatch(op: i32, cmd: &str, data: &[u8]) -> OpResult {
	match op {
		op::RUN => spawn(cmd, data, false, false),
		op::SHELL => spawn(cmd, data, false, true),
		op::EXEC => spawn(cmd, data, true, false),
		op::WHICH => which(cmd),
		op::ENV_ALL => env_all(),
		op::PID => OpResult::Bytes(std::process::id().to_string().into_bytes()),
		_ => OpResult::Err(format!("process: unknown op {op}")),
	}
}

/// The decoded request blob: an optional working directory, text piped to stdin, the
/// argument vector, and extra environment entries layered onto the inherited environment.
struct Req {
	cwd: Option<String>,
	stdin: Vec<u8>,
	args: Vec<String>,
	env: Vec<(String, String)>,
}

/// Parse the length-prefixed request blob `encode-req` builds. The blob is produced by our
/// own stdlib, so it is trusted: a malformed field reads back as empty rather than erroring.
fn parse_req(data: &[u8]) -> Req {
	let mut c = Cursor { b: data, pos: 0 };
	let cwd_raw = c.field();
	let cwd = if cwd_raw.is_empty() {
		None
	} else {
		Some(String::from_utf8_lossy(cwd_raw).into_owned())
	};
	let stdin = c.field().to_vec();
	let nargs = c.u32();
	let args = (0..nargs)
		.map(|_| String::from_utf8_lossy(c.field()).into_owned())
		.collect();
	let nenv = c.u32();
	let env = (0..nenv)
		.map(|_| {
			let k = String::from_utf8_lossy(c.field()).into_owned();
			let v = String::from_utf8_lossy(c.field()).into_owned();
			(k, v)
		})
		.collect();
	Req {
		cwd,
		stdin,
		args,
		env,
	}
}

/// A forward-only reader over the request blob. Big-endian `u32` lengths frame each field;
/// reads past the end clamp to empty (the blob is trusted, this is just panic-safety).
struct Cursor<'a> {
	b: &'a [u8],
	pos: usize,
}

impl<'a> Cursor<'a> {
	fn u32(&mut self) -> usize {
		if self.pos + 4 > self.b.len() {
			self.pos = self.b.len();
			return 0;
		}
		let v = u32::from_be_bytes(self.b[self.pos..self.pos + 4].try_into().unwrap());
		self.pos += 4;
		v as usize
	}

	fn field(&mut self) -> &'a [u8] {
		let n = self.u32();
		let end = (self.pos + n).min(self.b.len());
		let s = &self.b[self.pos..end];
		self.pos = end;
		s
	}
}

/// Spawn `cmd`, wait for it, and shape the outcome into an output blob. `inherit` wires the
/// child's stdio to the terminal (capturing nothing — for interactive tools); otherwise
/// stdout/stderr are captured and `req.stdin` is piped in. `shell` runs `cmd` as a
/// `/bin/sh -c` command line instead of an `argv[0]`. An `Err` here means the child could
/// not even start (command not found, permission denied) — a nonzero *exit* is a successful
/// run carried in the blob's status field.
fn spawn(cmd: &str, data: &[u8], inherit: bool, shell: bool) -> OpResult {
	let req = parse_req(data);
	let mut command = if shell {
		let mut c = Command::new("/bin/sh");
		c.arg("-c").arg(cmd);
		c
	} else {
		let mut c = Command::new(cmd);
		c.args(&req.args);
		c
	};
	if let Some(dir) = &req.cwd {
		command.current_dir(dir);
	}
	for (k, v) in &req.env {
		command.env(k, v);
	}

	if inherit {
		command
			.stdin(Stdio::inherit())
			.stdout(Stdio::inherit())
			.stderr(Stdio::inherit());
		return match command.status() {
			Ok(st) => OpResult::Bytes(encode_output(st.code().unwrap_or(-1), &[], &[])),
			Err(e) => OpResult::Err(e.to_string()),
		};
	}

	command
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped());
	let mut child = match command.spawn() {
		Ok(c) => c,
		Err(e) => return OpResult::Err(e.to_string()),
	};
	// Write stdin (if any) and close it, so a child reading to EOF unblocks even with no
	// input. Take()ing the handle and dropping it closes the pipe.
	if let Some(mut si) = child.stdin.take() {
		let _ = si.write_all(&req.stdin);
	}
	match child.wait_with_output() {
		Ok(out) => OpResult::Bytes(encode_output(
			out.status.code().unwrap_or(-1),
			&out.stdout,
			&out.stderr,
		)),
		Err(e) => OpResult::Err(e.to_string()),
	}
}

/// Frame a finished process as `[status: i32 BE][stdout: u32-len + bytes][stderr: u32-len +
/// bytes]`. `status` is signed (`-1` when the child was killed by a signal, no exit code).
fn encode_output(status: i32, stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
	let mut out = Vec::with_capacity(12 + stdout.len() + stderr.len());
	out.extend_from_slice(&status.to_be_bytes());
	out.extend_from_slice(&(stdout.len() as u32).to_be_bytes());
	out.extend_from_slice(stdout);
	out.extend_from_slice(&(stderr.len() as u32).to_be_bytes());
	out.extend_from_slice(stderr);
	out
}

/// Find `name` on `PATH`: the first entry that is a regular file with an executable bit set.
/// `Err` (→ `none` on the Pluma side) when nothing matches.
fn which(name: &str) -> OpResult {
	if let Some(paths) = std::env::var_os("PATH") {
		for dir in std::env::split_paths(&paths) {
			let cand = dir.join(name);
			if is_executable(&cand) {
				return OpResult::Bytes(cand.to_string_lossy().into_owned().into_bytes());
			}
		}
	}
	OpResult::Err(format!("{name}: not found on PATH"))
}

/// Snapshot the whole environment as `[count: u32][key-field][val-field]...`, each field a
/// `u32`-length-prefixed byte run — the framing the Pluma `decode-env` reads back.
fn env_all() -> OpResult {
	let vars: Vec<(String, String)> = std::env::vars().collect();
	let mut out = Vec::new();
	out.extend_from_slice(&(vars.len() as u32).to_be_bytes());
	for (k, v) in &vars {
		out.extend_from_slice(&(k.len() as u32).to_be_bytes());
		out.extend_from_slice(k.as_bytes());
		out.extend_from_slice(&(v.len() as u32).to_be_bytes());
		out.extend_from_slice(v.as_bytes());
	}
	OpResult::Bytes(out)
}

#[cfg(unix)]
fn is_executable(p: &std::path::Path) -> bool {
	use std::os::unix::fs::PermissionsExt;
	std::fs::metadata(p)
		.map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
		.unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &std::path::Path) -> bool {
	p.is_file()
}
