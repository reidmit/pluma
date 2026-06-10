// RPC endpoint metadata.
//
// A `public remote def` is the contract for a remote call. The contract is the
// pure logical signature `fun A.. -> task R` (no transport `request`): the
// compiler supplies each endpoint's body per target — a transport stub on the
// client, the written handler on the server — so end-to-end safety falls out of
// type-checking one signature against both the call site and the handler body.
//
// There are no generated source modules. The analyzer discovers each endpoint
// and records its resolved argument/result `wire` shapes here (`RpcEndpointMeta`);
// the lowerer reads that metadata to synthesize the client stub bodies and the
// `rpc-dispatch` routing table directly as IR (see `ir::lower`). This module just
// owns the metadata type and the per-route fingerprint.

use crate::ast::WireShape;

// What kind of remote call an endpoint is, decided by its return type:
// `fun A.. -> task R` is a unary request/response, `fun A.. -> stream R` is a
// server->client subscription. The kind drives both codegen (a transport stub vs
// a stream stub on the client; `respond` vs `respond-stream` on the server) and
// the fingerprint (so a unary client can't silently hit a stream route).
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum EndpointKind {
	Unary,
	Stream,
}

// One discovered `remote def`, with the resolved wire shapes the lowerer needs.
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct RpcEndpointMeta {
	// Full module name the def lives in (e.g. `api`, `shared.users`).
	pub module: String,
	// The def name (e.g. `add`).
	pub name: String,
	// Unary (`task R`) or stream (`stream R`).
	pub kind: EndpointKind,
	// Number of wire arguments (every parameter; there is no `request`).
	pub arity: usize,
	// The wire shape the arguments encode/decode as: the single parameter's
	// shape for arity 1, a `Tuple` of the parameters for arity ≥ 2, an empty
	// `Tuple` for arity 0 (nothing is sent). Encode and decode share it.
	pub arg_shape: WireShape,
	// The wire shape of the task's result `R`.
	pub result_shape: WireShape,
	// This route's own fingerprint (over its arg + result shapes). A stale
	// client calling a changed endpoint gets a `schema-skew` for that route
	// only; unrelated endpoints keep serving through a rolling deploy.
	pub route_fp: String,
}

impl RpcEndpointMeta {
	// The route the call travels over: `<module>.<name>`. Client and server
	// derive it identically; the path is `/rpc/<route>`.
	pub fn route(&self) -> String {
		format!("{}.{}", self.module, self.name)
	}
}

// A per-route fingerprint: a stable hash of the route's argument and result
// wire shapes. FNV-1a over a canonical rendering of each shape, masked to a
// non-negative `int`. A client and server built from the same endpoint types
// agree; a client built against drifted types does not.
pub fn fingerprint_shapes(kind: EndpointKind, arg: &WireShape, result: &WireShape) -> String {
	// The kind is part of the surface: a `task R` and a `stream R` with the same
	// shapes are different contracts, so they must fingerprint differently (a
	// unary client must not silently bind to a stream route, or vice versa). A
	// stream prefixes its line with `stream `; a unary keeps the bare shape line
	// (so existing unary fingerprints are unchanged, and the two are disjoint
	// because a rendered shape never begins with `stream `).
	let prefix = match kind {
		EndpointKind::Unary => "",
		EndpointKind::Stream => "stream ",
	};
	let line = format!("{}{}->{}", prefix, render_shape(arg), render_shape(result));
	let mut h: u64 = 0xcbf2_9ce4_8422_2325;
	for b in line.bytes() {
		h ^= b as u64;
		h = h.wrapping_mul(0x0000_0100_0000_01b3);
	}
	(h & (i64::MAX as u64)).to_string()
}

// A canonical, structural string for a resolved wire shape — the basis for the
// route fingerprint. Two shapes render identically iff they're the same wire
// schema, so the fingerprint changes exactly when an endpoint's wire surface
// drifts. `Var` cannot appear in a *resolved* endpoint shape (a free var fails
// the derivability check before this runs), so it renders as a placeholder.
fn render_shape(shape: &WireShape) -> String {
	use WireShape as W;
	match shape {
		W::Int => "int".to_string(),
		W::Float => "float".to_string(),
		W::Bool => "bool".to_string(),
		W::Str => "str".to_string(),
		W::Bytes => "bytes".to_string(),
		W::Duration => "duration".to_string(),
		W::Nothing => "nothing".to_string(),
		W::List(inner) => format!("[{}]", render_shape(inner)),
		W::Tuple(items) => {
			let xs: Vec<String> = items.iter().map(render_shape).collect();
			format!("({})", xs.join(","))
		}
		W::Dict(k, v) => format!("dict<{},{}>", render_shape(k), render_shape(v)),
		W::Record(fields) => {
			let fs: Vec<String> = fields
				.iter()
				.map(|(n, s)| format!("{}:{}", n, render_shape(s)))
				.collect();
			format!("{{{}}}", fs.join(","))
		}
		W::Enum {
			qualified,
			variants,
		} => {
			let vs: Vec<String> = variants
				.iter()
				.map(|(n, fields)| {
					let fs: Vec<String> = fields.iter().map(render_shape).collect();
					format!("{}[{}]", n, fs.join(","))
				})
				.collect();
			format!("enum {}<{}>", qualified, vs.join("|"))
		}
		W::EnumRef(qualified) => format!("ref {}", qualified),
		W::Var(_) => "?".to_string(),
	}
}
