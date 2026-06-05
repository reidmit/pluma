// FULLSTACK Layer 2 — RPC code generation.
//
// A `public remote def` is the contract for a remote call. From the set of
// remote defs reachable from a build's entry, we synthesize two Pluma source
// modules and inject them (`Compiler::generate_rpc_modules`, via
// `set_module_source`) so they flow through the normal analyze→lower pipeline:
//
//   * `rpc-client` — one stub per endpoint that encodes its arguments with
//     `wire`, POSTs them to `/rpc/<module>.<name>`, and decodes the reply into
//     the endpoint's `task` result. A settable `base-url` names the server.
//   * `rpc-server` — a `dispatch :: fun http.request -> task http.response`
//     that routes on the request path, decodes the arguments, calls the real
//     handler, and encodes the result. Mounted by the server's `main` with
//     `http.serve`.
//
// Generating Pluma *source* (rather than AST/IR) means the wire schemas
// auto-resolve through the constraint solver and the dispatcher type-checks
// against each handler's real signature — that *is* the end-to-end safety.

use crate::ast::{DefinitionKind, ModuleNode, TypeExprKind, TypeExprNode};

pub const CLIENT_MODULE: &str = "rpc-client";
pub const SERVER_MODULE: &str = "rpc-server";

// One discovered `remote def`.
pub struct Endpoint {
	// Full module name the def lives in (e.g. `api`, `shared.users`).
	pub module: String,
	// Local namespace the importer binds — the module's last segment (e.g.
	// `users`), used to call the handler as `<namespace>.<name>`.
	pub namespace: String,
	// The def name (e.g. `fetch`).
	pub name: String,
	// The def's type annotation — the contract, copied verbatim onto the stub.
	pub annotation: TypeExprNode,
	// Number of user arguments (the signature's parameters minus the leading
	// `request`).
	pub arity: usize,
}

impl Endpoint {
	// The route the call travels over: `/rpc/<module>.<name>`. Client and
	// server derive it identically.
	fn route(&self) -> String {
		format!("{}.{}", self.module, self.name)
	}
}

// Scan a parsed module's body for `remote def`s. Returns one `Endpoint` per
// well-formed endpoint; malformed signatures (caught separately by the
// analyzer's contract check) are skipped.
pub fn endpoints_in(module: &str, ast: &ModuleNode) -> Vec<Endpoint> {
	let namespace = module.rsplit('.').next().unwrap_or(module).to_string();
	let mut out = Vec::new();
	for def in &ast.body {
		if !def.is_remote {
			continue;
		}
		let DefinitionKind::Expr(_) = &def.kind else {
			continue;
		};
		let Some(annotation) = &def.type_annotation else {
			continue;
		};
		let Some(arity) = endpoint_arity(annotation) else {
			continue;
		};
		out.push(Endpoint {
			module: module.to_string(),
			namespace: namespace.clone(),
			name: def.name.name.clone(),
			annotation: annotation.clone(),
			arity,
		});
	}
	out
}

// User-argument count from an endpoint annotation `fun request A.. -> task R`:
// the parameter count minus the leading `request`. `None` if the annotation
// isn't a function type (the contract check reports that).
fn endpoint_arity(ann: &TypeExprNode) -> Option<usize> {
	match &ann.kind {
		TypeExprKind::Func(params, _) if !params.is_empty() => Some(params.len() - 1),
		TypeExprKind::Grouping(inner) => endpoint_arity(inner),
		_ => None,
	}
}

// The `rpc-client` module source: a settable base URL plus one stub per
// endpoint. The transport is the target's arm of the Phase 3 seam — `http.fetch`
// over `std.sys.net` on a sys/native build, the host page's `fetch.post` (the
// browser's `fetch`) on a web build.
pub fn generate_client(eps: &[Endpoint], fingerprint: &str, web: bool, base_url: &str) -> String {
	let mut s = String::new();
	s.push_str("# Generated RPC client stubs. Do not edit.\n");
	if web {
		s.push_str("use std.web.fetch\n");
	} else {
		s.push_str("use std.sys.http\n");
	}
	s.push_str("use std.task\n");
	s.push_str("use std.ref\n");
	s.push_str("use std.string\n");
	s.push_str("use std.dict\n");
	s.push_str("use std.request\n\n");

	// The server origin stubs POST to, baked in at build time (`pluma build/dev
	// --server-url`); empty means same-origin (`/rpc/...`, what `pluma dev` proxies).
	s.push_str(&format!(
		"def base-url :: ref string = ref.new \"{}\"\n\n",
		base_url.replace('\\', "\\\\").replace('"', "\\\"")
	));

	for ep in eps {
		let sig = render_type_expr(&ep.annotation);
		let params: Vec<String> = (0..ep.arity).map(|i| format!("a{}", i)).collect();
		let param_list = if params.is_empty() {
			String::new()
		} else {
			format!(" {}", params.join(" "))
		};
		// The request body: nothing to send for a zero-arg call; a single
		// encoded value, or an encoded tuple of all the arguments.
		let body = if ep.arity == 0 {
			"(string.to-bytes \"\")".to_string()
		} else if ep.arity == 1 {
			"(wire.encode a0)".to_string()
		} else {
			format!("(wire.encode ({}))", params.join(", "))
		};
		let route = ep.route();
		s.push_str(&format!(
			"public def {} :: {} = fun _req{} {{\n",
			ep.name, sig, param_list
		));
		// Every call carries the build's schema fingerprint as a header; the
		// dispatcher rejects a mismatch (a stale client) with a 409 the stub
		// turns into a clean transport failure — never a corrupt decode.
		s.push_str(&format!(
			"\tlet headers = dict.from-entries [(\"x-rpc-fingerprint\", \"{}\")]\n",
			fingerprint
		));
		// The transport call — same `task (result response string)` shape on both
		// targets, so the response handling below is identical.
		let fetch_call = if web {
			format!(
				"fetch.post (ref.get base-url ++ \"/rpc/{}\") headers {}",
				route, body
			)
		} else {
			format!(
				"http.fetch (ref.get base-url ++ \"/rpc/{}\") post headers {}",
				route, body
			)
		};
		s.push_str(&format!("\ttry resp = {}\n", fetch_call));
		s.push_str("\twhen resp is ok r {\n");
		s.push_str("\t\tif r.status == 409 {\n");
		s.push_str("\t\t\ttask.fail \"rpc: client out of date — reload\"\n");
		s.push_str("\t\t} else {\n");
		s.push_str("\t\t\twhen wire.decode r.body is ok value {\n");
		s.push_str("\t\t\t\ttask.return value\n");
		s.push_str("\t\t\t} is err _e {\n");
		s.push_str(&format!(
			"\t\t\t\ttask.fail \"rpc: malformed response from {}\"\n",
			route
		));
		s.push_str("\t\t\t}\n");
		s.push_str("\t\t}\n");
		s.push_str("\t} is err e {\n");
		s.push_str("\t\ttask.fail e\n");
		s.push_str("\t}\n}\n\n");
	}
	s
}

// The `rpc-server` module source: a single `dispatch` handler routing on the
// request path to each endpoint.
pub fn generate_server(eps: &[Endpoint], fingerprint: &str) -> String {
	let mut s = String::new();
	s.push_str("# Generated RPC dispatch table. Do not edit.\n");
	s.push_str("use std.sys.http\n");
	s.push_str("use std.task\n");
	s.push_str("use std.dict\n");
	s.push_str("use std.string\n");
	s.push_str("use std.request\n");
	let mut mods: Vec<&str> = eps.iter().map(|e| e.module.as_str()).collect();
	mods.sort();
	mods.dedup();
	for m in mods {
		s.push_str(&format!("use {}\n", m));
	}

	// The path-routing chain, built first, then nested inside the fingerprint
	// guard below.
	let mut routing = String::new();
	for (i, ep) in eps.iter().enumerate() {
		let route = ep.route();
		let head = if i == 0 { "if" } else { "} else if" };
		routing.push_str(&format!("{} req.path is \"/rpc/{}\" {{\n", head, route));

		let args: Vec<String> = (0..ep.arity).map(|i| format!("a{}", i)).collect();
		let call = if ep.arity == 0 {
			format!("{}.{} (request.new ())", ep.namespace, ep.name)
		} else {
			format!(
				"{}.{} (request.new ()) {}",
				ep.namespace,
				ep.name,
				args.join(" ")
			)
		};
		let ok_body =
			"\t\ttask.return {status: 200, headers: dict.empty (), body: wire.encode result}\n";

		if ep.arity == 0 {
			// No arguments: nothing to decode, just call the handler.
			routing.push_str(&format!("\ttry result = {}\n", call));
			routing.push_str(&ok_body.replace("\t\t", "\t"));
		} else {
			// Decode the argument(s). A tuple *pattern* closes the tuple so its
			// `wire` schema is derivable (`.0`/`.1` access leaves it open).
			let pat = if ep.arity == 1 {
				"a0".to_string()
			} else {
				format!("({})", args.join(", "))
			};
			routing.push_str(&format!("\twhen wire.decode req.body is ok {} {{\n", pat));
			routing.push_str(&format!("\t\ttry result = {}\n", call));
			routing.push_str(ok_body);
			routing.push_str("\t} is err _e {\n");
			routing.push_str("\t\ttask.return (http.text 400 \"rpc: malformed request\")\n");
			routing.push_str("\t}\n");
		}
	}
	if eps.is_empty() {
		routing.push_str("task.return (http.not-found ())\n");
	} else {
		routing.push_str("} else {\n");
		routing.push_str("\ttask.return (http.not-found ())\n");
		routing.push_str("}\n");
	}

	// Indent the routing one level so it sits inside the fingerprint guard.
	let routing: String = routing.lines().map(|l| format!("\t\t\t{}\n", l)).collect();

	// `dispatch` first checks the request's schema fingerprint against this
	// build's. A mismatch (a stale client built from drifted endpoint types)
	// gets a clean 409, never a corrupt positional decode.
	s.push_str("\npublic def dispatch :: fun http.request -> task http.response = fun req {\n");
	s.push_str("\twhen dict.lookup req.headers \"x-rpc-fingerprint\" is some fp {\n");
	s.push_str(&format!("\t\tif fp == \"{}\" {{\n", fingerprint));
	s.push_str(&routing);
	s.push_str("\t\t} else {\n");
	s.push_str("\t\t\ttask.return {status: 409, headers: dict.empty (), body: string.to-bytes \"rpc: schema mismatch\"}\n");
	s.push_str("\t\t}\n");
	s.push_str("\t} is none {\n");
	s.push_str("\t\ttask.return {status: 409, headers: dict.empty (), body: string.to-bytes \"rpc: missing schema fingerprint\"}\n");
	s.push_str("\t}\n");
	s.push_str("}\n");
	s
}

// A stable hash of the whole endpoint surface — every endpoint's qualified
// name and rendered signature, order-independent. Stamped identically into
// both generated modules so a client and server built from the same source
// agree, and a client built against drifted endpoint types does not. FNV-1a
// over the sorted signature lines, masked to a non-negative `int`.
pub fn surface_fingerprint(eps: &[Endpoint]) -> String {
	let mut lines: Vec<String> = eps
		.iter()
		.map(|e| {
			format!(
				"{}.{}::{}",
				e.module,
				e.name,
				render_type_expr(&e.annotation)
			)
		})
		.collect();
	lines.sort();
	let mut h: u64 = 0xcbf2_9ce4_8422_2325;
	for line in &lines {
		for b in line.bytes() {
			h ^= b as u64;
			h = h.wrapping_mul(0x0000_0100_0000_01b3);
		}
		h ^= 0x0a;
		h = h.wrapping_mul(0x0000_0100_0000_01b3);
	}
	(h & (i64::MAX as u64)).to_string()
}

// Render a type-expr back to Pluma source, for copying an endpoint's
// annotation onto its client stub. Covers the surface a `remote def`
// signature can use.
fn render_type_expr(t: &TypeExprNode) -> String {
	match &t.kind {
		TypeExprKind::Single(id) => {
			if id.generics.is_empty() {
				id.name.clone()
			} else {
				let args: Vec<String> = id.generics.iter().map(render_type_arg).collect();
				format!("{} {}", id.name, args.join(" "))
			}
		}
		TypeExprKind::Func(params, ret) => {
			let ps: Vec<String> = params.iter().map(render_type_arg).collect();
			format!("fun {} -> {}", ps.join(" "), render_type_expr(ret))
		}
		TypeExprKind::Tuple(items) => {
			let xs: Vec<String> = items.iter().map(render_type_expr).collect();
			format!("({})", xs.join(", "))
		}
		TypeExprKind::Record(fields) => {
			let fs: Vec<String> = fields
				.iter()
				.map(|(n, ft)| format!("{} :: {}", n.name, render_type_expr(ft)))
				.collect();
			format!("{{{}}}", fs.join(", "))
		}
		TypeExprKind::EmptyTuple => "()".to_string(),
		TypeExprKind::Grouping(inner) => format!("({})", render_type_expr(inner)),
	}
}

// A type in argument position needs parentheses when it's an applied type
// (`option int`) or a function — otherwise its arguments would re-associate.
fn render_type_arg(t: &TypeExprNode) -> String {
	match &t.kind {
		TypeExprKind::Single(id) if !id.generics.is_empty() => {
			format!("({})", render_type_expr(t))
		}
		TypeExprKind::Func(..) => format!("({})", render_type_expr(t)),
		_ => render_type_expr(t),
	}
}
