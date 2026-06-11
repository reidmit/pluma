// Host-import bookkeeping for `Module::build`: the import table (`HostImports`),
// the per-tag classification that decides which synthetic helpers + imports a
// builtin call pulls in (`classify_host_call`), and the tag -> wasm function type
// mapping for the import section (`import_type`).

use crate::Diagnostics;
use crate::helpers::helper_for_tag;
use crate::runtime::{
	ClockKind, DomKind, Helper, HelperSet, IoKind, RngKind, clock_kind, dom_kind, host_sig, io_kind,
	io_uses_io4, is_byte_writer, is_clock_host, is_dom_host, is_f64_unary_host, is_inline_builtin,
	is_io_host, is_io_result, is_raw_writer, is_rng_host, rng_kind, task_builtin_kind,
};
use crate::types::FuncTypes;
use std::collections::HashMap;

/// The program's host-import table: the `pluma`-module functions a reachable
/// program calls, in import order. `index` maps a tag to its wasm import index;
/// `order` is the dense list emitted into the import section. The two are kept in
/// lockstep — a tag's index always equals its position in `order` — by routing
/// every registration through [`HostImports::register`].
pub(super) struct HostImports {
	index: HashMap<String, u32>,
	order: Vec<String>,
}

impl HostImports {
	pub(super) fn new() -> Self {
		Self {
			index: HashMap::new(),
			order: Vec::new(),
		}
	}

	/// Register `name` as a host import and return its index. Idempotent: a tag
	/// already imported keeps its first-assigned index.
	pub(super) fn register(&mut self, name: &str) -> u32 {
		if let Some(&idx) = self.index.get(name) {
			return idx;
		}
		let idx = self.order.len() as u32;
		self.index.insert(name.to_string(), idx);
		self.order.push(name.to_string());
		idx
	}

	pub(super) fn get(&self, name: &str) -> Option<u32> {
		self.index.get(name).copied()
	}

	pub(super) fn contains(&self, name: &str) -> bool {
		self.index.contains_key(name)
	}

	/// Number of imports registered (the count occupying the low wasm indices).
	pub(super) fn len(&self) -> u32 {
		self.order.len() as u32
	}

	/// The import tags in order, for building the import section.
	pub(super) fn order(&self) -> &[String] {
		&self.order
	}

	/// The tag -> wasm-index map the emitter resolves host calls through.
	pub(super) fn index_map(&self) -> &HashMap<String, u32> {
		&self.index
	}
}

/// Classify a single non-net host call tag: request the synthetic helpers it
/// needs and register any host imports it implies. `is_net_builtin` tags are
/// handled by the caller (they register as a contiguous block after the scan), so
/// this is only ever called for non-net tags.
pub(super) fn classify_host_call(
	tag: &str,
	requested: &mut HelperSet,
	imports: &mut HostImports,
	diags: &mut Diagnostics,
) {
	if let Some(h) = helper_for_tag(tag) {
		requested.insert(h);
		return;
	}
	// `debug` is emitted inline (see `emit_debug`): it renders the value via
	// `__tostring`, concatenates the `[module:line]` prefix with `__bytesconcat`,
	// and prints the line through the `print` host import.
	if tag == "debug" {
		requested.insert(Helper::ToString);
		requested.insert(Helper::BytesConcat);
		requested.insert(Helper::MarshalSend);
		imports.register("print");
		return;
	}
	// Pure-compute builtins emitted inline at the call site (no import).
	if is_inline_builtin(tag) {
		return;
	}
	// `task.*` / `scope-new`/`scope-next` build a `$task` inline (no import); the
	// side-effecting scope-kernel ops call driver helpers — both are handled in
	// `emit`, never as host imports.
	if task_builtin_kind(tag).is_some()
		|| matches!(tag, "scope-spawn" | "scope-cancel" | "scope-cancel-after")
	{
		return;
	}
	// Unary float math: a `(f64) -> f64` host import (box/unbox emitted in wasm),
	// registered like any import but typed separately in `import_type`.
	if is_f64_unary_host(tag) {
		imports.register(tag);
		return;
	}
	// Byte-payload writers render their arg into scratch before the host call: they
	// need `__send_bytes` (and `__tostring` for the formatted ones). They still
	// register as ordinary host imports via the generic path below.
	if is_byte_writer(tag) {
		requested.insert(Helper::MarshalSend);
		if !is_raw_writer(tag) {
			requested.insert(Helper::ToString);
		}
	}
	// Marshalled `std/sys/io` ops encode their path/data args into scratch
	// (`__alloc`/`__store_bytes`); reads also `__load_bytes` the payload and need
	// the `io-copyout` overflow import, and `read-dir` splits names.
	if is_io_host(tag) {
		requested.insert(Helper::MarshalAlloc);
		requested.insert(Helper::MarshalStore);
		if let Some(kind) = io_kind(tag) {
			let is_read = matches!(
				kind,
				IoKind::ReadStr
					| IoKind::ReadBytes
					| IoKind::ReadFileStr
					| IoKind::ReadFileBytes
					| IoKind::ReadDir
					| IoKind::Args
					| IoKind::EnvVar
					| IoKind::FsOpSync
			);
			if is_read {
				requested.insert(Helper::MarshalLoad);
				imports.register("io-copyout");
			}
			// Both split a NUL-blob into a `$list` of `$str`.
			if matches!(kind, IoKind::ReadDir | IoKind::Args) {
				requested.insert(Helper::MarshalReadNames);
			}
			// `fs-op-sync` has no `host_sig` entry (its custom 7-arg type is handled in
			// `import_type`), so register it here rather than via the generic fallback below.
			if kind == IoKind::FsOpSync {
				imports.register("fs-op-sync");
			}
		}
	}

	// `std/sys/io` result builtins need the `__io_result` shaper + the
	// `io-last-error` channel it queries, on top of their own host import
	// (registered by the generic path below). `uuid-parse` rides this path too.
	if is_io_result(tag) {
		requested.insert(Helper::IoResult);
		imports.register("io-last-error");
	}

	// `std/random`/`std/uuid` payload builders (`emit_rng`): the byte/string ones
	// write to scratch and read it back (`random-bytes` may overflow); the scalars
	// need no helpers. Their host import is registered by the generic path below.
	if is_rng_host(tag) {
		match rng_kind(tag) {
			Some(RngKind::BytesN) => {
				requested.insert(Helper::MarshalAlloc);
				requested.insert(Helper::MarshalLoad);
				imports.register("io-copyout");
			}
			Some(RngKind::UuidStr) => {
				requested.insert(Helper::MarshalAlloc);
				requested.insert(Helper::MarshalLoad);
			}
			_ => {}
		}
	}

	// `std/time` clock imports (`emit_clock`). now/monotonic/sleep need no helpers;
	// `time-parse` marshals two strings + a scratch i64 slot and shapes its `result
	// instant string` through `__io_result` (so it needs the marshalling helpers +
	// the `io-last-error` error channel).
	if clock_kind(tag) == Some(ClockKind::Parse) {
		requested.insert(Helper::MarshalAlloc);
		requested.insert(Helper::MarshalStore);
		requested.insert(Helper::MarshalLoad);
		requested.insert(Helper::IoResult);
		imports.register("io-last-error");
	}

	// `std/web/dom` (`emit_dom`): string-carrying node ops marshal their args into
	// scratch; `dom-get-value` reads a payload back; `on-click` stows its handler in
	// the dispatch registry (the `__dom_register`/`__dom_dispatch` helpers, whose dep
	// `__list_push` and the `dom_handlers` global come in later). The dom host import
	// itself is registered by the generic path below.
	if is_dom_host(tag) {
		match dom_kind(tag) {
			Some(
				DomKind::Make
				| DomKind::SetText
				| DomKind::SetAttr
				| DomKind::SetProp
				| DomKind::SetBoolProp
				| DomKind::DevStoreSet,
			) => {
				requested.insert(Helper::MarshalAlloc);
				requested.insert(Helper::MarshalStore);
			}
			Some(DomKind::GetValue) => {
				requested.insert(Helper::MarshalAlloc);
				requested.insert(Helper::MarshalLoad);
			}
			// `dom-dev-store-get` marshals the key string out *and* reads the value
			// back, so it needs all three.
			Some(DomKind::DevStoreGet) => {
				requested.insert(Helper::MarshalAlloc);
				requested.insert(Helper::MarshalStore);
				requested.insert(Helper::MarshalLoad);
			}
			Some(DomKind::Listen) => {
				requested.insert(Helper::DomRegister);
				requested.insert(Helper::DomDispatch);
			}
			_ => {}
		}
	}

	// `std/web/fetch` under the V8 sys host: the blocking transport. Marshalled like an
	// io read (`(req_ptr, req_len, dst, cap) -> len`, overflow drained via `io-copyout`)
	// and shaped through `__io_result`; its own host import is registered by the generic
	// path below. (A browser build intercepts `web-fetch` in `module.rs` and routes it to
	// the async channel instead, so it never reaches this classifier.)
	if tag == "web-fetch" {
		requested.insert(Helper::MarshalAlloc);
		requested.insert(Helper::MarshalStore);
		requested.insert(Helper::MarshalLoad);
		requested.insert(Helper::IoResult);
		imports.register("io-last-error");
		imports.register("io-copyout");
	}
	if !imports.contains(tag) {
		if host_sig(tag).is_none() {
			diags.push(format!("unsupported host builtin `{tag}`"));
			return;
		}
		imports.register(tag);
	}
}

/// The wasm function type a host import `tag` resolves to (interned in `ftypes`).
/// Mirrors the host-side signatures the `host` runtime defines.
pub(super) fn import_type(tag: &str, ftypes: &mut FuncTypes) -> u32 {
	if tag == "float_to_str" {
		ftypes.for_float_to_str()
	} else if tag == "dom-set-timeout" {
		// `(i32 delay_ms, i32 token) -> ()` — the browser real-timer source.
		ftypes.for_dom_set_timeout()
	} else if is_byte_writer(tag) {
		ftypes.for_host_write()
	} else if tag == "io-copyout" || tag == "io-exit" {
		// Both are `(i32) -> ()`: io-copyout's `dst`, io-exit's `code`.
		ftypes.for_io_copyout()
	} else if tag == "io-last-error" {
		ftypes.for_io2()
	} else if tag == "fs-op-sync" {
		// `(op, pp, pl, dp, dl, dst, cap) -> len` — the sync fs op (7 args; neither io2 nor
		// io4, so handled before the generic io branch).
		ftypes.for_fs_op_sync()
	} else if tag == "web-fetch" {
		// `(req_ptr, req_len, dst, cap) -> len` — the sys host's blocking exchange, the
		// same shape as a path io read.
		ftypes.for_io4()
	} else if tag == "rpc-stream-open" || tag == "web-fetch-open" {
		// `(req_ptr, req_len, token) -> ()` — start a browser `fetch` (the subscription
		// reader, or the unary single-shot) keyed by the channel token.
		ftypes.for_rpc_stream_open()
	} else if tag == "rpc-stream-close" {
		// `(token) -> ()` — abort the subscription reader (shares io-copyout's `(i32)->()`).
		ftypes.for_io_copyout()
	} else if is_io_host(tag) {
		if io_uses_io4(tag) {
			ftypes.for_io4()
		} else {
			ftypes.for_io2()
		}
	} else if is_f64_unary_host(tag) {
		ftypes.for_f64_unary()
	} else if is_clock_host(tag) {
		match clock_kind(tag) {
			// now/monotonic: `() -> i64`, same shape as `random-int`.
			Some(ClockKind::NowInstant | ClockKind::MonotonicDuration) => ftypes.for_rng_i64(),
			Some(ClockKind::Sleep) => ftypes.for_time_sleep(),
			Some(ClockKind::Parse) | None => ftypes.for_time_parse(),
		}
	} else if is_rng_host(tag) {
		match rng_kind(tag) {
			Some(RngKind::ScalarI64) => ftypes.for_rng_i64(),
			Some(RngKind::ScalarF64) => ftypes.for_rng_f64(),
			Some(RngKind::RangeI64) => ftypes.for_rng_range(),
			Some(RngKind::BytesN) => ftypes.for_rng_bytes(),
			// uuid-v4/v7: `(dst, cap) -> len`, same shape as a two-arg io read.
			Some(RngKind::UuidStr) | None => ftypes.for_io2(),
		}
	} else if is_dom_host(tag) {
		match dom_kind(tag) {
			Some(DomKind::Body) => ftypes.for_dom_body(),
			Some(DomKind::Make) => ftypes.for_dom_make(),
			Some(DomKind::Append | DomKind::Append2) => ftypes.for_dom_append(),
			// `SetProp` shares `SetAttr`'s `(externref, np, nl, vp, vl)` shape;
			// `SetBoolProp` shares `Listen`'s `(externref, np, nl, i32)` shape.
			Some(DomKind::SetAttr | DomKind::SetProp) => ftypes.for_dom_set_attr(),
			Some(DomKind::SetText | DomKind::NodeStr) => ftypes.for_dom_node_str(),
			Some(DomKind::GetValue) => ftypes.for_dom_get_value(),
			Some(DomKind::Extern3) => ftypes.for_dom_extern3(),
			Some(DomKind::Extern1) => ftypes.for_dom_extern1(),
			Some(DomKind::ChildAt) => ftypes.for_dom_child_at(),
			Some(DomKind::DevStoreSet) => ftypes.for_dom_dev_store_set(),
			Some(DomKind::DevStoreGet) => ftypes.for_dom_dev_store_get(),
			Some(DomKind::Listen | DomKind::SetBoolProp) | None => ftypes.for_dom_listen(),
		}
	} else if tag == "net-listen" || tag == "net-accept" {
		ftypes.for_net_listen()
	} else if tag == "net-close" {
		ftypes.for_net_close()
	} else if tag == "net-local-addr" || tag == "net-connect" {
		// `net-connect` is now `(fid, addr_ptr, addr_len) -> (status, conn-id)` — the same
		// 3-in/2-out shape as `net-local-addr` (it's offloaded, so it takes the fiber id).
		ftypes.for_net_local_addr()
	} else if tag == "net-read" || tag == "net-write" {
		ftypes.for_net_rw()
	} else if tag == "io-poll" {
		ftypes.for_net_poll()
	} else if tag == "io-unwatch" {
		ftypes.for_net_unwatch()
	} else if tag == "offload-sleep" {
		ftypes.for_offload_sleep()
	} else if tag == "fs-op" {
		ftypes.for_offload_op()
	} else if tag == "db-op" {
		ftypes.for_db_op()
	} else {
		let sig = host_sig(tag).unwrap();
		ftypes.for_host(sig.arity, sig.returns_value)
	}
}
