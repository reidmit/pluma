// `core.dom` event-handler registry helpers (`Platform::Browser`).
//
// DOM events fire host-side, but a Pluma handler is a `$closure` heap value the
// host can't hold. So `dom.on-click` stows the handler in a module-level registry
// (`dom_handlers`, a `$list` of closures) and hands the host a bare i32 token —
// the closure's index. When the event fires the host calls the exported
// `__dom_dispatch(token)`, which looks the closure up and invokes it. Two helpers:
//   * `__dom_register(closure) -> token` — append + return the index (emit side).
//   * `__dom_dispatch(token) -> ()`      — the exported entry the host calls.

use wasm_encoder::{Function, ValType};

use crate::helpers::list::{list_len, mk_list};
use crate::helpers::wat::Wat;
use crate::types;

/// `__dom_register(closure) -> i32 token`: append `closure` to the `dom_handlers`
/// registry (lazily creating the empty `$list` the first time), returning its
/// pre-push index. `g` is the registry global; `list_push` the `__list_push` index.
pub(crate) fn build_dom_register_fn(g: u32, list_push: u32) -> Function {
	let mut w = Wat::new(1);
	let closure = w.param(0);
	let lst = w.local(types::value_ref());
	let arr = w.local(types::valarray_ref());
	let tok = w.local(ValType::I32);

	// Lazy init: if the registry is null, set it to a fresh empty `$list`.
	w.global_get(g).ref_is_null();
	w.if_(|w| {
		w.i32(0).array_new_default(types::T_VALARRAY).local_set(arr);
		mk_list(w, arr); // leaves the `$list` on the stack
		w.global_set(g);
	});
	// lst = the registry list; token = its current length (the new entry's index).
	w.global_get(g).local_set(lst);
	list_len(&mut w, lst);
	w.local_set(tok);
	// __list_push(lst, closure) — appends in place; discard its `nothing` result.
	w.local_get(lst).local_get(closure).call(list_push).drop();
	w.local_get(tok);
	w.finish()
}

/// `__dom_dispatch(i32 token) -> ()`: look up the handler closure at `token` in the
/// `dom_handlers` registry and invoke it (arity-1, with a `nothing` arg — the M1
/// handler type is `fun nothing -> nothing`). `g` is the registry global; `arity1`
/// the interned `(env, arg) -> value` closure type. Exported as `__dom_dispatch`.
pub(crate) fn build_dom_dispatch_fn(g: u32, arity1: u32) -> Function {
	let mut w = Wat::new(1);
	let token = w.param(0);
	let clos = w.local(types::value_ref());

	// clos = dom_handlers.elems[token].
	w.global_get(g)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1) // elems: $valarray
		.local_get(token)
		.array_get(types::T_VALARRAY)
		.local_set(clos);
	// Invoke arity-1: env = clos, arg0 = nothing, fn_index = clos.fn_index.
	w.local_get(clos).ref_cast(types::T_CLOSURE); // env (param 0)
	w.ref_null(types::T_VALUE); // arg0 = nothing (param 1)
	w.local_get(clos)
		.ref_cast(types::T_CLOSURE)
		.struct_get(types::T_CLOSURE, 1); // fn_index
	w.call_indirect(arity1).drop();
	w.finish()
}
