// Per-enum literal-table resolution for `Module::build`: the codecs and
// formatters (`__tostring`, `__dict_lookup`, the `*-compare` wrappers, the `wire`
// codec, the async driver, `__io_result`) dispatch on variant tags and emit
// variant display names, so each reachable one needs its enum's tags resolved and
// its display strings interned. Each block is gated on the feature being reachable
// and pushes a diagnostic if the prelude enum it needs is missing.

use crate::Diagnostics;
use crate::module::imports::HostImports;
use crate::runtime::{
	Helper, HelperSet, IoResultLits, OptionLits, OrderingLits, Runtime, TaskLits, ToStringLits,
	WireResultLits, WireTags,
};
use crate::scan::StrPool;
use ir::IrProgram;

/// The within-enum tag of variant `name` in the qualified enum `qual` (declaration
/// order = tag), if present.
fn tag_in(p: &IrProgram, qual: &str, name: &str) -> Option<u32> {
	p.enums
		.get(qual)
		.and_then(|vs| vs.iter().position(|(n, _)| n == name))
		.map(|i| i as u32)
}

/// Resolve every reachable feature's enum-literal table into `runtime`, interning
/// the display names it needs into `strpool`. Each block is independent; the order
/// here is the historical intern order (kept stable so the data segment is too).
pub(super) fn resolve_literals(
	p: &IrProgram,
	requested: &HelperSet,
	wrapper_order: &[String],
	imports: &HostImports,
	needs_wire_codec: bool,
	strpool: &mut StrPool,
	runtime: &mut Runtime,
	diags: &mut Diagnostics,
) {
	// `__tostring`'s fixed literals go in the data segment.
	if requested.contains(&Helper::ToString) {
		runtime.lits = ToStringLits {
			unit: strpool.intern("()"),
			tru: strpool.intern("true"),
			fals: strpool.intern("false"),
			lparen: strpool.intern("("),
			rparen: strpool.intern(")"),
			lbrack: strpool.intern("["),
			rbrack: strpool.intern("]"),
			lbrace: strpool.intern("{"),
			rbrace: strpool.intern("}"),
			comma_sp: strpool.intern(", "),
			colon_sp: strpool.intern(": "),
			space: strpool.intern(" "),
			ref_pfx: strpool.intern("ref "),
		};
	}
	// `__dict_lookup` builds `some v` / `none`; intern those variant display names
	// and resolve their within-enum tags (the `option` enum). `io.env` (`emit_env`)
	// builds the same `some`/`none` variants inline, and `__list_pop` returns
	// `some last` / `none`, so they need them populated too.
	if requested.contains(&Helper::DictLookup)
		|| requested.contains(&Helper::ListPop)
		|| imports.contains("io-env")
	{
		// Resolve `some`/`none` *within* the prelude `option` enum by its qualified name,
		// not by scanning every enum for those variant names — a user enum that happens to
		// declare a `some`/`none` variant must not steal (or, at a different tag position,
		// break) this resolution.
		let en = "__prelude__.option";
		let tag = |name: &str| {
			p.enums
				.get(en)
				.and_then(|vs| vs.iter().position(|(n, _)| n == name))
				.map(|i| i as u32)
		};
		match (tag("some"), tag("none")) {
			(Some(some_tag), Some(none_tag)) => {
				runtime.opt = OptionLits {
					some_tag,
					none_tag,
					some_gid: ir::global_ctor_id(&p.enums, en, some_tag).unwrap_or(0),
					none_gid: ir::global_ctor_id(&p.enums, en, none_tag).unwrap_or(0),
				};
			}
			_ => diags.push("building `some`/`none` needs the `option` enum".to_string()),
		}
	}
	// The `*-compare` wrappers build an `ordering` variant; intern its `lt`/`eq`/`gt`
	// display names and resolve their tags *within* the prelude `ordering` enum by its
	// qualified name. Resolving by qualified name (not a global scan for an enum with an
	// `lt` variant) is mandatory: a user/stdlib enum whose variants include `lt`/`eq`/`gt`
	// at other positions would otherwise make `variant_tag_in` ambiguous and break every
	// `compare` in the build (`std/sql`'s predicate enum hit exactly this).
	if wrapper_order.iter().any(|t| t.ends_with("-compare")) {
		let en = "__prelude__.ordering";
		let tag = |name: &str| {
			p.enums
				.get(en)
				.and_then(|vs| vs.iter().position(|(n, _)| n == name))
				.map(|i| i as u32)
		};
		match (tag("lt"), tag("eq"), tag("gt")) {
			(Some(lt_tag), Some(eq_tag), Some(gt_tag)) => {
				runtime.ord = OrderingLits {
					lt_tag,
					eq_tag,
					gt_tag,
					lt_gid: ir::global_ctor_id(&p.enums, en, lt_tag).unwrap_or(0),
					eq_gid: ir::global_ctor_id(&p.enums, en, eq_tag).unwrap_or(0),
					gt_gid: ir::global_ctor_id(&p.enums, en, gt_tag).unwrap_or(0),
				};
			}
			_ => diags.push("`compare` needs the `ordering` enum".to_string()),
		}
	}
	// The `wire` codec helpers dispatch on a schema node's `vtag`; resolve the
	// `wire-schema` enum's per-variant tags (declaration order = wire tag).
	if requested.contains(&Helper::WireFp) || needs_wire_codec {
		match p.enums.get("__prelude__.wire-schema") {
			Some(vs) => {
				let pos = |name: &str| vs.iter().position(|(n, _)| n == name).map(|i| i as u32);
				match (
					pos("s-int"),
					pos("s-float"),
					pos("s-bool"),
					pos("s-string"),
					pos("s-bytes"),
					pos("s-duration"),
					pos("s-nothing"),
					pos("s-list"),
					pos("s-dict"),
					pos("s-enum-ref"),
					pos("s-tuple"),
					pos("s-record"),
					pos("s-enum"),
				) {
					(
						Some(s_int),
						Some(s_float),
						Some(s_bool),
						Some(s_string),
						Some(s_bytes),
						Some(s_duration),
						Some(s_nothing),
						Some(s_list),
						Some(s_dict),
						Some(s_enum_ref),
						Some(s_tuple),
						Some(s_record),
						Some(s_enum),
					) => {
						runtime.wire = WireTags {
							s_int,
							s_float,
							s_bool,
							s_string,
							s_bytes,
							s_duration,
							s_nothing,
							s_list,
							s_dict,
							s_enum_ref,
							s_tuple,
							s_record,
							s_enum,
						};
					}
					_ => diags.push("`wire` needs the `wire-schema` enum variants".to_string()),
				}
			}
			None => diags.push("`wire` needs the `wire-schema` enum".to_string()),
		}
	}
	// `wire-decode` wraps its result in `ok`/`err`; resolve the `result` and
	// `wire-error` variant tags + display names `__wire_result` builds.
	if requested.contains(&Helper::WireDec) {
		let res = "__prelude__.result";
		let werr = "__prelude__.wire-error";
		match (tag_in(p, res, "ok"), tag_in(p, res, "err")) {
			(Some(ok_tag), Some(err_tag)) => {
				// `wire-error` variants, indexed by error code minus one.
				let err_names = [
					"unexpected-end",
					"invalid-tag",
					"invalid-utf8",
					"trailing-bytes",
					"malformed",
				];
				let mut errors = [(0u32, 0u32); 5];
				let mut ok = true;
				for (i, name) in err_names.iter().enumerate() {
					match tag_in(p, werr, name) {
						Some(t) => errors[i] = (t, ir::global_ctor_id(&p.enums, werr, t).unwrap_or(0)),
						None => ok = false,
					}
				}
				if ok {
					runtime.wirelits = WireResultLits {
						ok_tag,
						err_tag,
						ok_gid: ir::global_ctor_id(&p.enums, res, ok_tag).unwrap_or(0),
						err_gid: ir::global_ctor_id(&p.enums, res, err_tag).unwrap_or(0),
						errors,
					};
				} else {
					diags.push("`wire.decode` needs the `wire-error` enum variants".to_string());
				}
			}
			_ => diags.push("`wire.decode` needs the `result` enum".to_string()),
		}
	}
	// The async driver builds `result`/`option` variants (`task.attempt`, `s.next`,
	// root failure) and scans poll states for their `__defers` field. The driver runs
	// for every program, so these are always resolved.
	{
		let res = "__prelude__.result";
		let opt = "__prelude__.option";
		match (
			tag_in(p, res, "ok"),
			tag_in(p, res, "err"),
			tag_in(p, opt, "some"),
			tag_in(p, opt, "none"),
		) {
			(Some(ok_tag), Some(err_tag), Some(some_tag), Some(none_tag)) => {
				runtime.tasklits = TaskLits {
					ok_tag,
					err_tag,
					ok_gid: ir::global_ctor_id(&p.enums, res, ok_tag).unwrap_or(0),
					err_gid: ir::global_ctor_id(&p.enums, res, err_tag).unwrap_or(0),
					some_tag,
					none_tag,
					some_gid: ir::global_ctor_id(&p.enums, opt, some_tag).unwrap_or(0),
					none_gid: ir::global_ctor_id(&p.enums, opt, none_tag).unwrap_or(0),
					defers_name: strpool.intern("__defers"),
					cancelled_msg: strpool.intern("scope cancelled"),
					stream_fault_msg: strpool.intern("rpc.stream: stream faulted"),
					web_fetch_fail_msg: strpool.intern("web-fetch: request failed"),
				};
			}
			_ => diags.push("async runtime needs the `result` + `option` enums".to_string()),
		}
	}
	// `std/sys/io` result builtins wrap their host return in `ok`/`err` via
	// `__io_result`; resolve the `result` enum's variant tags + display names.
	if requested.contains(&Helper::IoResult) {
		let res = "__prelude__.result";
		match (tag_in(p, res, "ok"), tag_in(p, res, "err")) {
			(Some(ok_tag), Some(err_tag)) => {
				runtime.ioreslits = IoResultLits {
					ok_tag,
					err_tag,
					ok_gid: ir::global_ctor_id(&p.enums, res, ok_tag).unwrap_or(0),
					err_gid: ir::global_ctor_id(&p.enums, res, err_tag).unwrap_or(0),
				};
			}
			_ => diags.push("`std/sys/io` needs the `result` enum".to_string()),
		}
	}
}
