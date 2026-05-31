// The hand-emitted async runtime: the WasmGC analogue of `vm::task`. A cold
// `$task` (built by the task primitives + the async-fn lowering) is driven to
// completion by `__run_task`, a cooperative single-threaded scheduler over the
// CPS poll fns the VM runs Await-style (it snapshots an operand-stack frame the
// WasmGC has no addressable analogue for; see `ir::cps`).
//
// A CPS poll fn returns a `__poll` variant: `ready(value[, defers])` (vtag 0) or
// `pending(subtask, state')` (vtag 1). `__poll_step` calls it and reports
// completion vs suspension; the driver pushes a `Poll` activation on suspension
// and resumes it with the awaited value.
//
// Scheduler model (mirrors `vm::task`): the unit of execution is a *fiber* (an
// await chain belonging to a scope). `__pump` advances one fiber from a focus
// (Start/Ok/Err) over its activation stack until it completes or parks; the
// driver loop interleaves ready fibers, parks those waiting on a handle / scope /
// timer, and finalizes scopes once their body + every child have settled. State
// lives in module globals (`TaskGlobals`): the fiber/scope tables (`$list`s of
// `$tuple` field-records, indexed by id), the ready deque, and the pump's
// outcome/park output channel. The pumping fiber's activation chain is loaded
// into `act`/`actlen` for the duration of its pump, then saved back.
//
// Timers are VIRTUAL (no fixture observes wall-clock) — `__run_timers` jumps the
// logical clock to the earliest deadline. Stage-1 single-fiber fixtures fall out
// as the degenerate one-fiber case.

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::{Local, Wat};
use crate::runtime::sched::{NO_AWAITER, NO_SCOPE, ROOT_SCOPE, fiber, focus, outcome, scope, wait};
use crate::runtime::{TaskGlobals, TaskLits, act_kind, task_kind};
use crate::types;

// ==========================================================================
// Entry + the scheduler loop.
// ==========================================================================

/// `__task_entry(env) -> value`: the async program entry — call the real IR
/// entry (`main`), then drive the cold task it returns. Exported as `_entry`.
pub(crate) fn build_task_entry_fn(entry_idx: u32, run_task: u32) -> Function {
	let mut w = Wat::new(1);
	let env = w.param(0);
	w.local_get(env).call(entry_idx).call(run_task);
	w.finish()
}

/// `__run_task(root) -> value`: the scheduler loop. Seeds the root scope + fiber,
/// then drives ready fibers (running deferred cancellations between steps) until
/// the root settles. Returns the success value, or `result.err(e)` on root
/// failure (the harness reports it as a runtime error).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_run_task_fn(
	pump: u32,
	fiber_completed: u32,
	cancel_scope: u32,
	park: u32,
	run_timers: u32,
	list_append: u32,
	g: TaskGlobals,
	lits: TaskLits,
) -> Function {
	let v = types::value_ref();
	let mut w = Wat::new(1);
	let root = w.param(0);
	let fid = w.local(ValType::I32);
	let entry = w.local(v);

	// Reset scheduler state.
	empty_list(&mut w);
	w.global_set(g.pending);
	empty_list(&mut w);
	w.global_set(g.timers);
	w.i64(0).global_set(g.now);
	w.i32(0).global_set(g.root_kind);
	// scopes = [ root scope ]; fibers = [ root fiber ].
	empty_list(&mut w);
	push_scope(&mut w, ROOT_SCOPE, NO_AWAITER, 0); // body fid 0
	w.call(list_append).global_set(g.scopes);
	empty_list(&mut w);
	push_fiber(&mut w, ROOT_SCOPE, NO_SCOPE); // root fiber: not a scope body
	w.call(list_append).global_set(g.fibers);
	// ready = [ (0, Start, root) ].
	empty_list(&mut w);
	push_ready_entry(&mut w, 0, focus::START, |w| {
		w.local_get(root);
	});
	w.call(list_append).global_set(g.ready);
	w.i32(0).global_set(g.rhead);

	w.block("exit", |w| {
		w.loop_("sched", |w| {
			// Drain deferred cancellations (run `defer`s) before anything else.
			w.block("nocancel", |w| {
				w.loop_("cancels", |w| {
					list_len(w, g.pending);
					w.i32_eqz().br_if("nocancel");
					// sid = pending[len-1]; pending = drop-last.
					let sid = w.local(ValType::I32);
					w.global_get(g.pending)
						.ref_cast(types::T_LIST)
						.struct_get(types::T_LIST, 1);
					list_len(w, g.pending);
					w.i32(1).i32_sub().array_get(types::T_VALARRAY);
					unbox_i(w);
					w.local_set(sid);
					drop_last(w, g.pending);
					box_i(w, |w| {
						w.local_get(sid);
					});
					w.call(cancel_scope).drop();
					w.br("cancels");
				});
			});
			// Root settled?
			w.global_get(g.root_kind).br_if("exit");
			// A ready fiber?
			w.global_get(g.rhead);
			list_len(w, g.ready);
			w.i32_lt_s();
			w.if_else(
				|w| {
					// entry = ready[rhead]; rhead += 1.
					w.global_get(g.ready)
						.ref_cast(types::T_LIST)
						.struct_get(types::T_LIST, 1)
						.global_get(g.rhead)
						.array_get(types::T_VALARRAY)
						.local_set(entry);
					w.global_get(g.rhead).i32(1).i32_add().global_set(g.rhead);
					// fid = entry[0]; skip if dead.
					tuple_elem(w, entry, 0);
					unbox_i(w);
					w.local_set(fid);
					fld_i(w, g, g.fibers, fid, fiber::ALIVE);
					w.if_(|w| {
						// fiber.WAIT = none.
						set_fld_i(w, g.fibers, fid, fiber::WAIT_KIND, |w| {
							w.i32(wait::NONE);
						});
						// pump(fid, entry[1], entry[2]).
						box_i(w, |w| {
							w.local_get(fid);
						});
						box_i(w, |w| {
							tuple_elem(w, entry, 1);
							unbox_i(w);
						});
						tuple_elem(w, entry, 2);
						w.call(pump);
						w.drop();
						// done?
						w.global_get(g.out_kind).i32(1).i32_eq();
						w.if_else(
							|w| {
								box_i(w, |w| {
									w.local_get(fid);
								});
								box_i(w, |w| {
									w.global_get(g.out_okerr);
								});
								w.global_get(g.out_val);
								w.call(fiber_completed).drop();
							},
							|w| {
								// park(fid, wait_kind, arg). Sleep's nanos ride the i64
								// channel; the other waits pass a small id on `out_arg`.
								box_i(w, |w| {
									w.local_get(fid);
								});
								box_i(w, |w| {
									w.global_get(g.out_okerr);
								});
								w.global_get(g.out_okerr).i32(wait::SLEEP).i32_eq();
								w.if_result(
									types::value_ref(),
									|w| {
										box_i64(w, |w| {
											w.global_get(g.out_arg64);
										});
									},
									|w| {
										box_i(w, |w| {
											w.global_get(g.out_arg);
										});
									},
								);
								w.call(park).drop();
							},
						);
					});
				},
				|w| {
					// Nothing ready: fire timers, else quiesce.
					list_len(w, g.timers);
					w.if_else(
						|w| {
							// Fire the earliest virtual timer(s) (advances the clock).
							w.call(run_timers).drop();
						},
						|w| {
							w.br("exit");
						},
					);
				},
			);
			w.br("sched");
		});
	});

	// Decode the root outcome: ok -> value, err -> result.err(e), cancelled -> ().
	let _ = list_append;
	w.global_get(g.root_kind).i32(outcome::ERR).i32_eq();
	w.if_result(
		v,
		|w| {
			push_result(w, lits.err_tag, lits.err_name, |w| {
				w.global_get(g.root_val);
			});
		},
		|w| {
			w.global_get(g.root_kind).i32(outcome::OK).i32_eq();
			w.if_result(
				v,
				|w| {
					w.global_get(g.root_val);
				},
				push_nothing,
			);
		},
	);
	w.finish()
}

// ==========================================================================
// The per-fiber driver (`__pump`).
// ==========================================================================

/// `__pump(fid, fkind, fval) -> nothing`: advance fiber `fid` from focus
/// `(fkind, fval)` until it completes or parks, writing the result to the output
/// globals (`out_kind` 1 = done / 2 = park; `out_okerr` = outcome/ wait kind;
/// `out_val`/`out_arg` = the payload). Mirrors `vm::task::pump` + `advance_one`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_pump_fn(
	poll_step: u32,
	poll_defers_state: u32,
	act_push: u32,
	start_scope: u32,
	drain_next: u32,
	arity1: u32,
	g: TaskGlobals,
	lits: TaskLits,
) -> Function {
	let v = types::value_ref();
	let va = types::valarray_ref();
	let mut w = Wat::new(3);
	let (fid_b, fkind_b, fval0) = (w.param(0), w.param(1), w.param(2));
	let fid = w.local(ValType::I32);
	let fkind = w.local(ValType::I32);
	let fval = w.local(v);
	let tk = w.local(ValType::I32);
	let tp = w.local(va);
	let a = w.local(v);
	let akind = w.local(ValType::I32);
	let apl = w.local(va);
	let ps = w.local(v);
	let pspl = w.local(va);
	let dn = w.local(v);
	let psk = w.local(ValType::I32);
	let pc = w.local(v);
	let child = w.local(ValType::I32);
	let ck = w.local(ValType::I32);
	// Shield depth: while > 0 the running region is `task.shielded`, so its
	// yield/sleep continue inline (no park) and a sibling can't interleave.
	let shield = w.local(ValType::I32);

	w.i32(0).local_set(shield);
	w.local_get(fid_b);
	unbox_i(&mut w);
	w.local_set(fid);
	w.local_get(fkind_b);
	unbox_i(&mut w);
	w.local_set(fkind);
	w.local_get(fval0).local_set(fval);

	// Load the fiber's activation chain into the working stack (a fresh copy).
	load_act(&mut w, g, fid);

	w.block("ret", |w| {
		w.loop_("main", |w| {
			// ---- Start: `fval` is a `$task`; dispatch its kind. -----------------
			w.local_get(fkind).i32_eqz();
			w.if_(|w| {
				w.local_get(fval)
					.ref_cast(types::T_TASK)
					.struct_get(types::T_TASK, 1)
					.local_set(tk);
				w.local_get(fval)
					.ref_cast(types::T_TASK)
					.struct_get(types::T_TASK, 2)
					.local_set(tp);

				start_settle(
					w,
					tk,
					task_kind::PURE,
					focus::OK,
					|w| elem(w, tp, 0),
					fval,
					fkind,
				);
				start_settle(
					w,
					tk,
					task_kind::FAIL,
					focus::ERR,
					|w| elem(w, tp, 0),
					fval,
					fkind,
				);
				start_combinator(
					w,
					tk,
					task_kind::THEN,
					act_kind::THEN,
					tp,
					true,
					act_push,
					fval,
					fkind,
				);
				start_combinator(
					w,
					tk,
					task_kind::ORELSE,
					act_kind::ORELSE,
					tp,
					true,
					act_push,
					fval,
					fkind,
				);
				start_combinator(
					w,
					tk,
					task_kind::ATTEMPT,
					act_kind::ATTEMPT,
					tp,
					false,
					act_push,
					fval,
					fkind,
				);
				start_combinator(
					w,
					tk,
					task_kind::MAP,
					act_kind::MAP,
					tp,
					true,
					act_push,
					fval,
					fkind,
				);

				// shielded: run the inner uninterruptibly — bump the depth, mark the
				// region end on the chain, and run the inner inline.
				w.local_get(tk).i32(task_kind::SHIELDED).i32_eq();
				w.if_(|w| {
					w.local_get(shield).i32(1).i32_add().local_set(shield);
					push_activation(w, act_kind::SHIELD, push_nothing, push_nothing);
					w.call(act_push).drop();
					elem(w, tp, 0);
					w.local_set(fval);
					w.i32(focus::START).local_set(fkind);
					w.br("main");
				});

				// yield: shielded -> continue inline; else re-ready behind everyone.
				w.local_get(tk).i32(task_kind::YIELD).i32_eq();
				w.if_(|w| {
					w.local_get(shield).i32(0).i32_gt_s();
					w.if_else(
						|w| {
							push_nothing(w);
							w.local_set(fval);
							w.i32(focus::OK).local_set(fkind);
							w.br("main");
						},
						|w| {
							save_act(w, g, fid);
							park_out(w, g, wait::YIELD, |w| {
								w.i32(0);
							});
							w.br("ret");
						},
					);
				});

				// sleep: shielded -> continue inline; else park on a virtual timer.
				w.local_get(tk).i32(task_kind::SLEEP).i32_eq();
				w.if_(|w| {
					w.local_get(shield).i32(0).i32_gt_s();
					w.if_else(
						|w| {
							push_nothing(w);
							w.local_set(fval);
							w.i32(focus::OK).local_set(fkind);
							w.br("main");
						},
						|w| {
							save_act(w, g, fid);
							w.i32(2).global_set(g.out_kind);
							w.i32(wait::SLEEP).global_set(g.out_okerr);
							elem(w, tp, 0);
							w.ref_cast(types::T_INT).struct_get(types::T_INT, 1); // nanos (i64)
							w.global_set(g.out_arg64);
							w.br("ret");
						},
					);
				});

				// async: advance the CPS poll fn one step.
				w.local_get(tk).i32(task_kind::ASYNC).i32_eq();
				w.if_(|w| {
					elem(w, tp, 0);
					w.local_set(pc);
					w.local_get(pc);
					elem(w, tp, 1);
					push_nothing(w);
					w.call(poll_step).local_set(ps);
					poll_after(w, pc, ps, pspl, psk, fval, fkind, act_push);
				});

				// scope: create the scope + body fiber, park on it.
				w.local_get(tk).i32(task_kind::SCOPE).i32_eq();
				w.if_(|w| {
					// start_scope(fid, manual, body_fn) -> sid.
					box_i(w, |w| {
						w.local_get(fid);
					});
					elem(w, tp, 0); // manual (bool)
					elem(w, tp, 1); // body_fn
					w.call(start_scope);
					unbox_i(w);
					let sid = w.local(ValType::I32);
					w.local_set(sid);
					save_act(w, g, fid);
					park_out(w, g, wait::SCOPE, |w| {
						w.local_get(sid);
					});
					w.br("ret");
				});

				// handle: await a spawned child fiber.
				w.local_get(tk).i32(task_kind::HANDLE).i32_eq();
				w.if_(|w| {
					elem(w, tp, 0);
					unbox_i(w);
					w.local_set(child);
					fld_i(w, g, g.fibers, child, fiber::RES_KIND);
					w.local_tee(ck);
					w.if_else(
						|w| {
							// settled: ok -> Ok(val), err -> Err(val), cancelled -> Ok(()).
							w.local_get(ck).i32(outcome::OK).i32_eq();
							w.if_else(
								|w| {
									fld(w, g, g.fibers, child, fiber::RES_VAL);
									w.local_set(fval);
									w.i32(focus::OK).local_set(fkind);
								},
								|w| {
									w.local_get(ck).i32(outcome::ERR).i32_eq();
									w.if_else(
										|w| {
											fld(w, g, g.fibers, child, fiber::RES_VAL);
											w.local_set(fval);
											w.i32(focus::ERR).local_set(fkind);
										},
										|w| {
											push_nothing(w);
											w.local_set(fval);
											w.i32(focus::OK).local_set(fkind);
										},
									);
								},
							);
							w.br("main");
						},
						|w| {
							save_act(w, g, fid);
							park_out(w, g, wait::HANDLE, |w| {
								w.local_get(child);
							});
							w.br("ret");
						},
					);
				});

				// next: drain the manual scope (`s.next`).
				w.local_get(tk).i32(task_kind::NEXT).i32_eq();
				w.if_(|w| {
					// drain_next(handle) -> (action, val). The handle is tp[0].
					elem(w, tp, 0);
					w.call(drain_next).local_set(dn);
					w.local_get(dn)
						.ref_cast(types::T_TUPLE)
						.struct_get(types::T_TUPLE, 1);
					w.i32(0).array_get(types::T_VALARRAY);
					unbox_i(w);
					w.i32_eqz(); // action == 0 -> produce, else park.
					w.if_else(
						|w| {
							w.local_get(dn)
								.ref_cast(types::T_TUPLE)
								.struct_get(types::T_TUPLE, 1);
							w.i32(1).array_get(types::T_VALARRAY);
							w.local_set(fval);
							w.i32(focus::OK).local_set(fkind);
							w.br("main");
						},
						|w| {
							save_act(w, g, fid);
							park_out(w, g, wait::NEXT, |w| {
								elem(w, tp, 0);
								unbox_i(w);
							});
							w.br("ret");
						},
					);
				});

				w.unreachable();
			});

			// ---- Ok: settle a value down the activation chain. ------------------
			w.local_get(fkind).i32(focus::OK).i32_eq();
			w.if_(|w| {
				w.loop_("ok", |w| {
					w.global_get(g.actlen).i32_eqz();
					w.if_(|w| {
						done_out(w, g, outcome::OK, |w| {
							w.local_get(fval);
						});
						save_act(w, g, fid);
						w.br("ret");
					});
					pop_activation(w, g, a, akind, apl);
					w.local_get(akind).i32(act_kind::POLL).i32_eq();
					w.if_(|w| {
						elem(w, apl, 0);
						w.local_set(pc);
						w.local_get(pc);
						elem(w, apl, 1);
						w.local_get(fval);
						w.call(poll_step).local_set(ps);
						poll_after(w, pc, ps, pspl, psk, fval, fkind, act_push);
					});
					w.local_get(akind).i32(act_kind::THEN).i32_eq();
					w.if_(|w| {
						call1(
							w,
							|w| elem(w, apl, 0),
							|w| {
								w.local_get(fval);
							},
							arity1,
						);
						w.local_set(fval);
						w.i32(focus::START).local_set(fkind);
						w.br("main");
					});
					w.local_get(akind).i32(act_kind::ORELSE).i32_eq();
					w.if_(|w| {
						w.br("ok");
					});
					w.local_get(akind).i32(act_kind::ATTEMPT).i32_eq();
					w.if_(|w| {
						push_result(w, lits.ok_tag, lits.ok_name, |w| {
							w.local_get(fval);
						});
						w.local_set(fval);
						w.br("ok");
					});
					w.local_get(akind).i32(act_kind::MAP).i32_eq();
					w.if_(|w| {
						call1(
							w,
							|w| elem(w, apl, 0),
							|w| {
								w.local_get(fval);
							},
							arity1,
						);
						w.local_set(fval);
						w.br("ok");
					});
					// shield region end: leave the shielded region, keep settling.
					w.local_get(akind).i32(act_kind::SHIELD).i32_eq();
					w.if_(|w| {
						w.local_get(shield).i32(1).i32_sub().local_set(shield);
						w.br("ok");
					});
					w.unreachable();
				});
			});

			// ---- Err: propagate a failure down the activation chain. -------------
			w.local_get(fkind).i32(focus::ERR).i32_eq();
			w.if_(|w| {
				w.loop_("err", |w| {
					w.global_get(g.actlen).i32_eqz();
					w.if_(|w| {
						done_out(w, g, outcome::ERR, |w| {
							w.local_get(fval);
						});
						save_act(w, g, fid);
						w.br("ret");
					});
					pop_activation(w, g, a, akind, apl);
					w.local_get(akind).i32(act_kind::POLL).i32_eq();
					w.if_(|w| {
						elem(w, apl, 1);
						w.call(poll_defers_state).drop();
						w.br("err");
					});
					w.local_get(akind).i32(act_kind::THEN).i32_eq();
					w.if_(|w| {
						w.br("err");
					});
					w.local_get(akind).i32(act_kind::MAP).i32_eq();
					w.if_(|w| {
						w.br("err");
					});
					w.local_get(akind).i32(act_kind::ORELSE).i32_eq();
					w.if_(|w| {
						call1(w, |w| elem(w, apl, 0), push_nothing, arity1);
						w.local_set(fval);
						w.i32(focus::START).local_set(fkind);
						w.br("main");
					});
					w.local_get(akind).i32(act_kind::ATTEMPT).i32_eq();
					w.if_(|w| {
						push_result(w, lits.err_tag, lits.err_name, |w| {
							w.local_get(fval);
						});
						w.local_set(fval);
						w.i32(focus::OK).local_set(fkind);
						w.br("main");
					});
					// shield region end: leave the shielded region, keep propagating.
					w.local_get(akind).i32(act_kind::SHIELD).i32_eq();
					w.if_(|w| {
						w.local_get(shield).i32(1).i32_sub().local_set(shield);
						w.br("err");
					});
					w.unreachable();
				});
			});

			w.unreachable();
		});
	});
	push_nothing(&mut w);
	w.finish()
}

// ==========================================================================
// Scope/fiber lifecycle (mirrors `vm::task`).
// ==========================================================================

/// `__start_scope(fid, manual, body_fn) -> sid (boxed)`: create a scope owned by
/// `fid`, run `body_fn(handle)` as its root fiber, and return the new scope id.
pub(crate) fn build_start_scope_fn(list_append: u32, arity1: u32, g: TaskGlobals) -> Function {
	let v = types::value_ref();
	let mut w = Wat::new(3);
	let (fid_b, manual, body_fn) = (w.param(0), w.param(1), w.param(2));
	let sid = w.local(ValType::I32);
	let bf = w.local(ValType::I32);
	let body_task = w.local(v);

	// sid = |scopes|. scopes.append(new scope { manual, awaiter=fid, body=0 }).
	// (BODY is patched below — calling a *non-async* body runs its `s.spawn`s now,
	// appending child fibers, so the body fiber's index isn't known until after.)
	list_len(&mut w, g.scopes);
	w.local_set(sid);
	w.global_get(g.scopes);
	w.i32(types::TAG_TUPLE);
	scope_fields(
		&mut w,
		|w| {
			w.local_get(manual)
				.ref_cast(types::T_BOOL)
				.struct_get(types::T_BOOL, 1);
		},
		|w| {
			w.i32(0);
		},
		|w| {
			w.local_get(fid_b)
				.ref_cast(types::T_INT)
				.struct_get(types::T_INT, 1)
				.i32_wrap_i64();
		},
	);
	w.struct_new(types::T_TUPLE);
	w.call(list_append).global_set(g.scopes);

	// body_task = body_fn(ScopeHandle(sid)).
	call1(
		&mut w,
		|w| {
			w.local_get(body_fn);
		},
		|w| {
			w.i32(types::TAG_SCOPE_HANDLE);
			w.local_get(sid);
			w.i64_extend_i32_s();
			w.struct_new(types::T_INT);
		},
		arity1,
	);
	w.local_set(body_task);

	// bf = |fibers| (now, after any spawns). Append the body fiber, patch BODY.
	list_len(&mut w, g.fibers);
	w.local_set(bf);
	w.global_get(g.fibers);
	w.i32(types::TAG_TUPLE);
	fiber_fields(
		&mut w,
		|w| {
			w.local_get(sid);
		},
		|w| {
			w.local_get(sid);
		},
	);
	w.struct_new(types::T_TUPLE);
	w.call(list_append).global_set(g.fibers);
	set_fld_i(&mut w, g.scopes, sid, scope::BODY, |w| {
		w.local_get(bf);
	});

	// ready.append((bf, Start, body_task)).
	ready_push(&mut w, g, list_append, bf, focus::START, |w| {
		w.local_get(body_task);
	});

	box_i(&mut w, |w| {
		w.local_get(sid);
	});
	w.finish()
}

/// `__sched_spawn(handle, task) -> handle-task`: start `task` as a child of the
/// handle's scope and return a `HANDLE` task awaiting it. Called by `s.spawn`.
pub(crate) fn build_sched_spawn_fn(list_append: u32, g: TaskGlobals) -> Function {
	let mut w = Wat::new(2);
	let (handle, task) = (w.param(0), w.param(1));
	let sid = w.local(ValType::I32);
	let fid = w.local(ValType::I32);

	w.local_get(handle)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(sid);
	list_len(&mut w, g.fibers);
	w.local_set(fid);

	// fibers.append(new child fiber { scope=sid, runs_scope=none }).
	w.global_get(g.fibers);
	w.i32(types::TAG_TUPLE);
	fiber_fields(
		&mut w,
		|w| {
			w.local_get(sid);
		},
		|w| {
			w.i32(NO_SCOPE as i32);
		},
	);
	w.struct_new(types::T_TUPLE);
	w.call(list_append).global_set(g.fibers);

	// scope.children.append(fid).
	set_fld(&mut w, g.scopes, sid, scope::CHILDREN, |w| {
		fld(w, g, g.scopes, sid, scope::CHILDREN);
		box_i(w, |w| {
			w.local_get(fid);
		});
		w.call(list_append);
	});
	// ready.append((fid, Start, task)).
	ready_push(&mut w, g, list_append, fid, focus::START, |w| {
		w.local_get(task);
	});

	// return HANDLE task carrying the child fid.
	w.i32(types::TAG_TASK);
	w.i32(task_kind::HANDLE);
	box_i(&mut w, |w| {
		w.local_get(fid);
	});
	w.array_new_fixed(types::T_VALARRAY, 1);
	w.struct_new(types::T_TASK);
	w.finish()
}

/// `__fiber_completed(fid, kind, val) -> nothing`: a fiber settled. Route its
/// outcome: root sets the program result; a scope body finalizes its scope; a
/// spawned child wakes its waiters and may trip fail-fast.
pub(crate) fn build_fiber_completed_fn(
	on_body_done: u32,
	on_child_done: u32,
	g: TaskGlobals,
) -> Function {
	let mut w = Wat::new(3);
	let (fid_b, kind_b, val) = (w.param(0), w.param(1), w.param(2));
	let fid = w.local(ValType::I32);
	let kind = w.local(ValType::I32);
	let rs = w.local(ValType::I32);

	w.local_get(fid_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(fid);
	w.local_get(kind_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(kind);

	set_fld_i(&mut w, g.fibers, fid, fiber::ALIVE, |w| {
		w.i32(0);
	});
	set_fld_i(&mut w, g.fibers, fid, fiber::RES_KIND, |w| {
		w.local_get(kind);
	});
	set_fld(&mut w, g.fibers, fid, fiber::RES_VAL, |w| {
		w.local_get(val);
	});

	// root?
	w.local_get(fid).i32_eqz();
	w.if_else(
		|w| {
			w.i32(0).global_set(g.out_kind); // unused
			w.local_get(kind).global_set(g.root_kind);
			w.local_get(val).global_set(g.root_val);
		},
		|w| {
			fld_i(w, g, g.fibers, fid, fiber::RUNS_SCOPE);
			w.local_tee(rs);
			w.i32(NO_SCOPE as i32).i32_ne();
			w.if_else(
				|w| {
					// scope body finished.
					box_i(w, |w| {
						w.local_get(rs);
					});
					box_i(w, |w| {
						w.local_get(kind);
					});
					w.local_get(val);
					w.call(on_body_done).drop();
				},
				|w| {
					// spawned child finished.
					fld_i(w, g, g.fibers, fid, fiber::SCOPE);
					box_i_top(w);
					box_i(w, |w| {
						w.local_get(fid);
					});
					box_i(w, |w| {
						w.local_get(kind);
					});
					w.local_get(val);
					w.call(on_child_done).drop();
				},
			);
		},
	);
	push_nothing(&mut w);
	w.finish()
}

/// `__on_body_done(sid, kind, val) -> nothing`: a scope's body fiber settled.
pub(crate) fn build_on_body_done_fn(
	reap_fiber: u32,
	try_finalize: u32,
	g: TaskGlobals,
) -> Function {
	let mut w = Wat::new(3);
	let (sid_b, kind_b, val) = (w.param(0), w.param(1), w.param(2));
	let sid = w.local(ValType::I32);
	let kind = w.local(ValType::I32);
	let children = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let c = w.local(ValType::I32);

	w.local_get(sid_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(sid);
	w.local_get(kind_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(kind);

	set_fld_i(&mut w, g.scopes, sid, scope::BD_KIND, |w| {
		w.local_get(kind);
	});
	set_fld(&mut w, g.scopes, sid, scope::BD_VAL, |w| {
		w.local_get(val);
	});
	// A failing non-manual body fails the scope.
	w.local_get(kind).i32(outcome::ERR).i32_eq();
	fld_i(&mut w, g, g.scopes, sid, scope::MANUAL);
	w.i32_eqz();
	w.i32_and();
	fld_i(&mut w, g, g.scopes, sid, scope::FAIL_SET);
	w.i32_eqz();
	w.i32_and();
	w.if_(|w| {
		set_fld_i(w, g.scopes, sid, scope::FAIL_SET, |w| {
			w.i32(1);
		});
		set_fld(w, g.scopes, sid, scope::FAIL_VAL, |w| {
			w.local_get(val);
		});
	});
	// Cancel any still-running children (the structural guarantee).
	fld(&mut w, g, g.scopes, sid, scope::CHILDREN);
	w.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(children);
	w.local_get(children).array_len().local_set(n);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			w.local_get(children)
				.local_get(i)
				.array_get(types::T_VALARRAY);
			unbox_i(w);
			w.local_set(c);
			fld_i(w, g, g.fibers, c, fiber::ALIVE);
			w.if_(|w| {
				box_i(w, |w| {
					w.local_get(c);
				});
				w.call(reap_fiber).drop();
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(sid_b).call(try_finalize).drop();
	push_nothing(&mut w);
	w.finish()
}

/// `__on_child_done(sid, fid, kind, val) -> nothing`: a spawned child settled.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_on_child_done_fn(
	cancel_scope: u32,
	try_finalize: u32,
	list_append: u32,
	g: TaskGlobals,
	lits: TaskLits,
) -> Function {
	let mut w = Wat::new(4);
	let (sid_b, fid_b, kind_b, val) = (w.param(0), w.param(1), w.param(2), w.param(3));
	let sid = w.local(ValType::I32);
	let fid = w.local(ValType::I32);
	let kind = w.local(ValType::I32);
	let waiters = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let wfid = w.local(ValType::I32);
	let observed = w.local(ValType::I32);

	w.local_get(sid_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(sid);
	w.local_get(fid_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(fid);
	w.local_get(kind_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(kind);

	// Deliver to waiters; clear them.
	fld(&mut w, g, g.fibers, fid, fiber::WAITERS);
	w.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(waiters);
	w.local_get(waiters).array_len().local_tee(n);
	w.i32(0).i32_gt_s().local_set(observed);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			w.local_get(waiters)
				.local_get(i)
				.array_get(types::T_VALARRAY);
			unbox_i(w);
			w.local_set(wfid);
			// ready.append((wfid, focus-of(kind), val-or-nothing)).
			ready_push_outcome(w, g, list_append, wfid, kind, val, lits);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	set_fld(&mut w, g.fibers, fid, fiber::WAITERS, empty_list);
	// Feed `s.next`: hand straight to a parked drainer, else queue for later.
	let nw = w.local(types::valarray_ref());
	let nwn = w.local(ValType::I32);
	let nwfid = w.local(ValType::I32);
	let octmp = w.local(types::value_ref());
	fld(&mut w, g, g.scopes, sid, scope::NEXT_WAITERS);
	w.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(nw);
	w.local_get(nw).array_len().local_tee(nwn).i32(0).i32_gt_s();
	w.if_else(
		|w| {
			w.local_get(nw).i32(0).array_get(types::T_VALARRAY);
			unbox_i(w);
			w.local_set(nwfid);
			set_fld(w, g.scopes, sid, scope::NEXT_WAITERS, |w| {
				drop_first_list(w, nw, nwn);
			});
			mk_outcome(w, kind, val);
			w.local_set(octmp);
			ready_push(w, g, list_append, nwfid, focus::OK, |w| {
				push_some(w, lits, |w| push_settled(w, lits, octmp));
			});
		},
		|w| {
			set_fld(w, g.scopes, sid, scope::COMPLETED, |w| {
				fld(w, g, g.scopes, sid, scope::COMPLETED);
				mk_outcome(w, kind, val);
				w.call(list_append);
			});
		},
	);

	// Fail-fast: an unobserved failure in a live non-manual scope cancels it.
	w.local_get(kind).i32(outcome::ERR).i32_eq();
	w.local_get(observed).i32_eqz();
	w.i32_and();
	fld_i(&mut w, g, g.scopes, sid, scope::MANUAL);
	w.i32_eqz();
	w.i32_and();
	fld_i(&mut w, g, g.scopes, sid, scope::CANCELLED);
	w.i32_eqz();
	w.i32_and();
	fld_i(&mut w, g, g.scopes, sid, scope::FAIL_SET);
	w.i32_eqz();
	w.i32_and();
	w.if_else(
		|w| {
			set_fld_i(w, g.scopes, sid, scope::FAIL_SET, |w| {
				w.i32(1);
			});
			set_fld(w, g.scopes, sid, scope::FAIL_VAL, |w| {
				w.local_get(val);
			});
			w.local_get(sid_b).call(cancel_scope).drop();
		},
		|w| {
			w.local_get(sid_b).call(try_finalize).drop();
		},
	);
	push_nothing(&mut w);
	w.finish()
}

/// `__cancel_scope(sid) -> nothing`: cancel a scope + everything it owns.
pub(crate) fn build_cancel_scope_fn(
	reap_fiber: u32,
	try_finalize: u32,
	g: TaskGlobals,
) -> Function {
	let mut w = Wat::new(1);
	let sid_b = w.param(0);
	let sid = w.local(ValType::I32);
	let body = w.local(ValType::I32);
	let children = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let c = w.local(ValType::I32);

	w.local_get(sid_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(sid);

	w.block("done", |w| {
		// Already cancelled or finalized?
		fld_i(w, g, g.scopes, sid, scope::CANCELLED);
		fld_i(w, g, g.scopes, sid, scope::FINALIZED);
		w.i32_or().br_if("done");
		set_fld_i(w, g.scopes, sid, scope::CANCELLED, |w| {
			w.i32(1);
		});
		// Reap the body; if it never ran, mark its outcome cancelled.
		fld_i(w, g, g.scopes, sid, scope::BODY);
		w.local_set(body);
		fld_i(w, g, g.fibers, body, fiber::ALIVE);
		w.if_(|w| {
			box_i(w, |w| {
				w.local_get(body);
			});
			w.call(reap_fiber).drop();
			fld_i(w, g, g.scopes, sid, scope::BD_KIND);
			w.i32_eqz();
			w.if_(|w| {
				set_fld_i(w, g.scopes, sid, scope::BD_KIND, |w| {
					w.i32(outcome::CANCELLED);
				});
			});
		});
		// Reap every live child.
		fld(w, g, g.scopes, sid, scope::CHILDREN);
		w.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(children);
		w.local_get(children).array_len().local_set(n);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
				w.local_get(children)
					.local_get(i)
					.array_get(types::T_VALARRAY);
				unbox_i(w);
				w.local_set(c);
				fld_i(w, g, g.fibers, c, fiber::ALIVE);
				w.if_(|w| {
					box_i(w, |w| {
						w.local_get(c);
					});
					w.call(reap_fiber).drop();
				});
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		w.local_get(sid_b).call(try_finalize).drop();
	});
	push_nothing(&mut w);
	w.finish()
}

/// `__reap_fiber(fid) -> nothing`: abandon a parked/queued fiber — cascade into a
/// sub-scope it awaited, run its `defer` cleanups, and mark it cancelled.
pub(crate) fn build_reap_fiber_fn(
	cancel_scope: u32,
	poll_defers_state: u32,
	g: TaskGlobals,
) -> Function {
	let v = types::value_ref();
	let mut w = Wat::new(1);
	let fid_b = w.param(0);
	let fid = w.local(ValType::I32);
	let act = w.local(types::valarray_ref());
	let i = w.local(ValType::I32);
	let act_el = w.local(v);

	w.local_get(fid_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(fid);
	w.block("done", |w| {
		fld_i(w, g, g.fibers, fid, fiber::ALIVE);
		w.i32_eqz().br_if("done");
		set_fld_i(w, g.fibers, fid, fiber::ALIVE, |w| {
			w.i32(0);
		});
		set_fld_i(w, g.fibers, fid, fiber::RES_KIND, |w| {
			w.i32(outcome::CANCELLED);
		});
		// If it was awaiting a sub-scope, cancel that too.
		fld_i(w, g, g.fibers, fid, fiber::WAIT_KIND);
		w.i32(wait::SCOPE).i32_eq();
		w.if_(|w| {
			box_i(w, |w| {
				fld_i(w, g, g.fibers, fid, fiber::WAIT_ARG);
			});
			w.call(cancel_scope).drop();
		});
		// Run the fiber's poll `defer`s, innermost (top of stack) first.
		fld(w, g, g.fibers, fid, fiber::ACT);
		w.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(act);
		w.local_get(act).array_len().i32(1).i32_sub().local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).i32(0).i32_lt_s().br_if("brk");
				w.local_get(act)
					.local_get(i)
					.array_get(types::T_VALARRAY)
					.local_set(act_el);
				// activation kind == POLL ? run its state's defers.
				w.local_get(act_el)
					.ref_cast(types::T_VARIANT)
					.struct_get(types::T_VARIANT, 1);
				w.i32(act_kind::POLL).i32_eq();
				w.if_(|w| {
					w.local_get(act_el)
						.ref_cast(types::T_VARIANT)
						.struct_get(types::T_VARIANT, 3);
					w.i32(1).array_get(types::T_VALARRAY); // state
					w.call(poll_defers_state).drop();
				});
				w.local_get(i).i32(1).i32_sub().local_set(i);
				w.br("lp");
			});
		});
	});
	push_nothing(&mut w);
	w.finish()
}

/// `__try_finalize_scope(sid) -> nothing`: finalize once the body + every child
/// have settled; wake the awaiter with the scope's result (fail-fast wins).
pub(crate) fn build_try_finalize_scope_fn(
	list_append: u32,
	g: TaskGlobals,
	lits: TaskLits,
) -> Function {
	let v = types::value_ref();
	let mut w = Wat::new(1);
	let sid_b = w.param(0);
	let sid = w.local(ValType::I32);
	let children = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let aw = w.local(ValType::I32);
	let rkind = w.local(ValType::I32);
	let rval = w.local(v);

	w.local_get(sid_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(sid);
	w.block("done", |w| {
		fld_i(w, g, g.scopes, sid, scope::FINALIZED);
		w.br_if("done");
		// Body done?
		fld_i(w, g, g.scopes, sid, scope::BD_KIND);
		w.i32_eqz().br_if("done");
		// All children done?
		fld(w, g, g.scopes, sid, scope::CHILDREN);
		w.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(children);
		w.local_get(children).array_len().local_set(n);
		w.i32(0).local_set(i);
		w.block("allok", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("allok");
				w.local_get(children)
					.local_get(i)
					.array_get(types::T_VALARRAY);
				unbox_i(w);
				let c = w.local(ValType::I32);
				w.local_set(c);
				fld_i(w, g, g.fibers, c, fiber::ALIVE);
				w.br_if("done"); // a child still alive -> not yet
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		set_fld_i(w, g.scopes, sid, scope::FINALIZED, |w| {
			w.i32(1);
		});
		// result = fail-fast failure, else body outcome.
		fld_i(w, g, g.scopes, sid, scope::FAIL_SET);
		w.if_else(
			|w| {
				w.i32(outcome::ERR).local_set(rkind);
				fld(w, g, g.scopes, sid, scope::FAIL_VAL);
				w.local_set(rval);
			},
			|w| {
				fld_i(w, g, g.scopes, sid, scope::BD_KIND);
				w.local_set(rkind);
				fld(w, g, g.scopes, sid, scope::BD_VAL);
				w.local_set(rval);
			},
		);
		// Wake the awaiter, if any and alive.
		fld_i(w, g, g.scopes, sid, scope::AWAITER);
		w.local_tee(aw);
		w.i32(NO_AWAITER as i32).i32_ne();
		w.if_(|w| {
			fld_i(w, g, g.fibers, aw, fiber::ALIVE);
			w.if_(|w| {
				set_fld_i(w, g.fibers, aw, fiber::WAIT_KIND, |w| {
					w.i32(wait::NONE);
				});
				// cancelled scope -> recoverable failure with the cancelled message.
				w.local_get(rkind).i32(outcome::CANCELLED).i32_eq();
				w.if_else(
					|w| {
						ready_push(w, g, list_append, aw, focus::ERR, |w| {
							str_lit(w, lits.cancelled_msg);
						});
					},
					|w| {
						// ok -> Ok(val), err -> Err(val).
						w.local_get(rkind).i32(outcome::OK).i32_eq();
						w.if_else(
							|w| {
								ready_push(w, g, list_append, aw, focus::OK, |w| {
									w.local_get(rval);
								});
							},
							|w| {
								ready_push(w, g, list_append, aw, focus::ERR, |w| {
									w.local_get(rval);
								});
							},
						);
					},
				);
			});
		});
	});
	push_nothing(&mut w);
	w.finish()
}

/// `__park(fid, wait_kind, wait_arg) -> nothing`: register a parked fiber against
/// what it's waiting on.
pub(crate) fn build_park_fn(list_append: u32, g: TaskGlobals) -> Function {
	let mut w = Wat::new(3);
	let (fid_b, wk_b, wa_b) = (w.param(0), w.param(1), w.param(2));
	let fid = w.local(ValType::I32);
	let wk = w.local(ValType::I32);
	let wa = w.local(ValType::I32);
	let wa64 = w.local(ValType::I64);

	w.local_get(fid_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(fid);
	w.local_get(wk_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(wk);
	// The arg arrives boxed as an i64: a small id for handle/next/scope, or sleep nanos.
	w.local_get(wa_b)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.local_tee(wa64)
		.i32_wrap_i64()
		.local_set(wa);

	w.block("after", |w| {
		// yield: re-ready behind everything else.
		w.local_get(wk).i32(wait::YIELD).i32_eq();
		w.if_(|w| {
			ready_push(w, g, list_append, fid, focus::OK, push_nothing);
			w.br("after");
		});
		// sleep: arm a virtual timer to re-ready the fiber at `now + nanos`.
		w.local_get(wk).i32(wait::SLEEP).i32_eq();
		w.if_(|w| {
			w.global_get(g.timers);
			timer_entry(
				w,
				|w| {
					w.global_get(g.now).local_get(wa64).i64_add();
				},
				0,
				|w| {
					w.local_get(fid);
				},
			);
			w.call(list_append).global_set(g.timers);
		});
		// handle: enqueue on the awaited child's waiters.
		w.local_get(wk).i32(wait::HANDLE).i32_eq();
		w.if_(|w| {
			set_fld(w, g.fibers, wa, fiber::WAITERS, |w| {
				fld(w, g, g.fibers, wa, fiber::WAITERS);
				box_i(w, |w| {
					w.local_get(fid);
				});
				w.call(list_append);
			});
		});
		// next: enqueue on the scope's `s.next` waiter list (the scope `wa`).
		w.local_get(wk).i32(wait::NEXT).i32_eq();
		w.if_(|w| {
			set_fld(w, g.scopes, wa, scope::NEXT_WAITERS, |w| {
				fld(w, g, g.scopes, wa, scope::NEXT_WAITERS);
				box_i(w, |w| {
					w.local_get(fid);
				});
				w.call(list_append);
			});
		});
		// scope: nothing — the scope wakes its awaiter on finalize. (sleep is 2c.)
		set_fld_i(w, g.fibers, fid, fiber::WAIT_KIND, |w| {
			w.local_get(wk);
		});
		set_fld_i(w, g.fibers, fid, fiber::WAIT_ARG, |w| {
			w.local_get(wa);
		});
	});
	push_nothing(&mut w);
	w.finish()
}

/// `__run_timers() -> nothing`: VIRTUAL timers — jump the clock to the earliest
/// deadline and fire every timer due at it. `Wake` re-readies a live fiber;
/// `Deadline` queues a scope cancellation. No wall-clock wait.
pub(crate) fn build_run_timers_fn(list_append: u32, g: TaskGlobals) -> Function {
	let v = types::value_ref();
	let va = types::valarray_ref();
	let mut w = Wat::new(0);
	let arr = w.local(va);
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let min = w.local(ValType::I64);
	let entry = w.local(v);
	let at = w.local(ValType::I64);
	let kind = w.local(ValType::I32);
	let arg = w.local(ValType::I32);
	let newt = w.local(v);

	w.global_get(g.timers)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(arr);
	w.local_get(arr).array_len().local_set(n);
	// min = earliest `at`.
	w.i64(i64::MAX).local_set(min);
	w.i32(0).local_set(i);
	w.block("mbrk", |w| {
		w.loop_("mlp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("mbrk");
			timer_at(w, arr, i);
			w.local_tee(at).local_get(min).i64_lt_s();
			w.if_(|w| {
				w.local_get(at).local_set(min);
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("mlp");
		});
	});
	w.local_get(min).global_set(g.now);
	// Fire all timers at `min`; keep the rest.
	empty_list(&mut w);
	w.local_set(newt);
	w.i32(0).local_set(i);
	w.block("fbrk", |w| {
		w.loop_("flp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("fbrk");
			w.local_get(arr)
				.local_get(i)
				.array_get(types::T_VALARRAY)
				.local_set(entry);
			tuple_elem(w, entry, 0);
			w.ref_cast(types::T_INT)
				.struct_get(types::T_INT, 1)
				.local_set(at);
			w.local_get(at).local_get(min).i64_eq();
			w.if_else(
				|w| {
					tuple_elem(w, entry, 1);
					unbox_i(w);
					w.local_set(kind);
					tuple_elem(w, entry, 2);
					unbox_i(w);
					w.local_set(arg);
					w.local_get(kind).i32_eqz();
					w.if_else(
						|w| {
							// Wake: re-ready the fiber if still alive.
							fld_i(w, g, g.fibers, arg, fiber::ALIVE);
							w.if_(|w| {
								set_fld_i(w, g.fibers, arg, fiber::WAIT_KIND, |w| {
									w.i32(wait::NONE);
								});
								ready_push(w, g, list_append, arg, focus::OK, push_nothing);
							});
						},
						|w| {
							// Deadline: queue the scope cancellation.
							w.global_get(g.pending);
							box_i(w, |w| {
								w.local_get(arg);
							});
							w.call(list_append).global_set(g.pending);
						},
					);
				},
				|w| {
					w.local_get(newt);
					w.local_get(entry);
					w.call(list_append);
					w.local_set(newt);
				},
			);
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("flp");
		});
	});
	w.local_get(newt).global_set(g.timers);
	push_nothing(&mut w);
	w.finish()
}

/// `__sched_cancel(handle, _) -> nothing`: `s.cancel` — queue the scope for
/// cancellation, performed between scheduler steps (so `defer`s run there).
pub(crate) fn build_sched_cancel_fn(list_append: u32, g: TaskGlobals) -> Function {
	let mut w = Wat::new(2);
	let handle = w.param(0);
	w.global_get(g.pending);
	box_i(&mut w, |w| {
		w.local_get(handle)
			.ref_cast(types::T_INT)
			.struct_get(types::T_INT, 1)
			.i32_wrap_i64();
	});
	w.call(list_append).global_set(g.pending);
	push_nothing(&mut w);
	w.finish()
}

/// `__sched_cancel_after(handle, duration) -> nothing`: `s.cancel-after` — arm a
/// deadline timer that self-cancels the scope once `duration` elapses.
pub(crate) fn build_sched_cancel_after_fn(list_append: u32, g: TaskGlobals) -> Function {
	let mut w = Wat::new(2);
	let (handle, dur) = (w.param(0), w.param(1));
	let sid = w.local(ValType::I32);
	w.local_get(handle)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(sid);
	w.global_get(g.timers);
	timer_entry(
		&mut w,
		|w| {
			w.global_get(g.now);
			w.local_get(dur)
				.ref_cast(types::T_INT)
				.struct_get(types::T_INT, 1);
			w.i64_add();
		},
		1,
		|w| {
			w.local_get(sid);
		},
	);
	w.call(list_append).global_set(g.timers);
	push_nothing(&mut w);
	w.finish()
}

/// Push timer entry `i`'s `at` field (i64) from the timers `$valarray`.
fn timer_at(w: &mut Wat, arr: Local, i: Local) {
	w.local_get(arr).local_get(i).array_get(types::T_VALARRAY);
	w.ref_cast(types::T_TUPLE)
		.struct_get(types::T_TUPLE, 1)
		.i32(0)
		.array_get(types::T_VALARRAY);
	w.ref_cast(types::T_INT).struct_get(types::T_INT, 1);
}

/// `__drain_next(handle) -> $tuple(action, val)`: `s.next` — hand back the next
/// settled child as `some (ok/err …)`, `none` once every child has drained, or
/// signal a park. `action` 0 = produce `val` (Ok focus), 1 = park on `Next`.
pub(crate) fn build_drain_next_fn(g: TaskGlobals, lits: TaskLits) -> Function {
	let v = types::value_ref();
	let va = types::valarray_ref();
	let mut w = Wat::new(1);
	let handle = w.param(0);
	let sid = w.local(ValType::I32);
	let comp = w.local(va);
	let n = w.local(ValType::I32);
	let oc = w.local(v);

	w.local_get(handle)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(sid);
	fld(&mut w, g, g.scopes, sid, scope::COMPLETED);
	w.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(comp);
	w.local_get(comp).array_len().local_tee(n).i32(0).i32_gt_s();
	w.if_result(
		v,
		|w| {
			// A settled child is queued: pop the front, yield `some (settled)`.
			w.local_get(comp)
				.i32(0)
				.array_get(types::T_VALARRAY)
				.local_set(oc);
			set_fld(w, g.scopes, sid, scope::COMPLETED, |w| {
				drop_first_list(w, comp, n);
			});
			action_tuple(w, 0, |w| push_some(w, lits, |w| push_settled(w, lits, oc)));
		},
		|w| {
			all_children_done(w, g, sid);
			w.if_result(
				v,
				|w| action_tuple(w, 0, |w| push_none(w, lits)),
				|w| action_tuple(w, 1, push_nothing),
			);
		},
	);
	w.finish()
}

/// `__list_append(list, elem) -> list`: a fresh `$list` of `list`'s elements then
/// `elem`. (O(n) rebuild — the scheduler's collections are tiny.)
pub(crate) fn build_list_append_fn(arrconcat: u32) -> Function {
	let mut w = Wat::new(2);
	let (list, el) = (w.param(0), w.param(1));
	w.i32(types::TAG_LIST);
	w.local_get(list)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1);
	w.local_get(el);
	w.array_new_fixed(types::T_VALARRAY, 1);
	w.call(arrconcat);
	w.struct_new(types::T_LIST);
	w.finish()
}

// ==========================================================================
// CPS poll machinery (fiber-agnostic; unchanged from Stage 1).
// ==========================================================================

/// `__poll_step(pc, state, resume) -> $tuple(kind, x, y)`: advance one CPS poll.
pub(crate) fn build_poll_step_fn(poll_defers_list: u32, arity2: u32) -> Function {
	let v = types::value_ref();
	let va = types::valarray_ref();
	let mut w = Wat::new(3);
	let (pc, state, resume) = (w.param(0), w.param(1), w.param(2));
	let r = w.local(v);
	let rpl = w.local(va);

	w.local_get(pc).ref_cast(types::T_CLOSURE);
	w.local_get(state);
	w.local_get(resume);
	w.local_get(pc)
		.ref_cast(types::T_CLOSURE)
		.struct_get(types::T_CLOSURE, 1);
	w.call_indirect(arity2);
	w.local_set(r);

	w.local_get(r)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 3)
		.local_set(rpl);
	w.local_get(r)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 1)
		.i32_eqz();
	w.if_result(
		v,
		|w| {
			w.local_get(rpl).array_len().i32(2).i32_ge_s();
			w.if_(|w| {
				elem(w, rpl, 1);
				w.call(poll_defers_list).drop();
			});
			push_tuple3(w, 0, |w| elem(w, rpl, 0), push_nothing);
		},
		|w| {
			push_tuple3(w, 1, |w| elem(w, rpl, 0), |w| elem(w, rpl, 1));
		},
	);
	w.finish()
}

/// `__poll_defers_list(list) -> nothing`: run a `$list` of zero-arg cleanup
/// closures LIFO (the CPS pass appends, so back to front).
pub(crate) fn build_poll_defers_list_fn(arity1: u32) -> Function {
	let v = types::value_ref();
	let mut w = Wat::new(1);
	let list = w.param(0);
	let arr = w.local(types::valarray_ref());
	let i = w.local(ValType::I32);
	let c = w.local(v);

	w.local_get(list)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(arr);
	w.local_get(arr).array_len().i32(1).i32_sub().local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).i32(0).i32_lt_s().br_if("brk");
			w.local_get(arr)
				.local_get(i)
				.array_get(types::T_VALARRAY)
				.local_set(c);
			call1(
				w,
				|w| {
					w.local_get(c);
				},
				push_nothing,
				arity1,
			);
			w.drop();
			w.local_get(i).i32(1).i32_sub().local_set(i);
			w.br("lp");
		});
	});
	push_nothing(&mut w);
	w.finish()
}

/// `__poll_defers_state(state) -> nothing`: run the `__defers` cleanup list
/// carried in a suspended poll state, if present (tolerant of its absence).
pub(crate) fn build_poll_defers_state_fn(
	eq: u32,
	poll_defers_list: u32,
	defers_name: (u32, u32),
) -> Function {
	let v = types::value_ref();
	let va = types::valarray_ref();
	let mut w = Wat::new(1);
	let state = w.param(0);
	let names = w.local(va);
	let vals = w.local(va);
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let key = w.local(v);

	str_lit(&mut w, defers_name);
	w.local_set(key);
	w.local_get(state)
		.ref_cast(types::T_RECORD)
		.struct_get(types::T_RECORD, 1)
		.local_set(names);
	w.local_get(state)
		.ref_cast(types::T_RECORD)
		.struct_get(types::T_RECORD, 2)
		.local_set(vals);
	w.local_get(names).array_len().local_set(n);
	w.i32(0).local_set(i);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			w.local_get(names).local_get(i).array_get(types::T_VALARRAY);
			w.local_get(key);
			w.call(eq);
			w.if_(|w| {
				w.local_get(vals).local_get(i).array_get(types::T_VALARRAY);
				w.call(poll_defers_list).drop();
				w.br("brk");
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	push_nothing(&mut w);
	w.finish()
}

/// `__act_push(activation) -> nothing`: push one activation onto the current
/// fiber's working stack, growing the backing `$valarray` when full.
pub(crate) fn build_act_push_fn(g: TaskGlobals) -> Function {
	let mut w = Wat::new(1);
	let act = w.param(0);
	let cap = w.local(ValType::I32);
	let na = w.local(types::valarray_ref());
	let src = w.local(types::valarray_ref_null());

	w.global_get(g.act).array_len().local_set(cap);
	w.global_get(g.actlen).local_get(cap).i32_ge_s();
	w.if_(|w| {
		w.local_get(cap).i32_eqz();
		w.if_result(
			ValType::I32,
			|w| {
				w.i32(16);
			},
			|w| {
				w.local_get(cap).i32(2).i32_mul();
			},
		);
		w.array_new_default(types::T_VALARRAY).local_set(na);
		w.global_get(g.act).local_set(src);
		w.copy_loop(types::T_VALARRAY, na, None, src, None, cap);
		w.local_get(na).global_set(g.act);
	});
	w.global_get(g.act)
		.global_get(g.actlen)
		.local_get(act)
		.array_set(types::T_VALARRAY);
	w.global_get(g.actlen).i32(1).i32_add().global_set(g.actlen);
	push_nothing(&mut w);
	w.finish()
}

// ==========================================================================
// Shared emission fragments.
// ==========================================================================

/// Push the `i`-th element of a `$valarray` local.
fn elem(w: &mut Wat, arr: Local, i: i32) {
	w.local_get(arr).i32(i).array_get(types::T_VALARRAY);
}

/// Push the `i`-th element of a `$tuple` local (cast + index its elems).
fn tuple_elem(w: &mut Wat, tup: Local, i: i32) {
	w.local_get(tup)
		.ref_cast(types::T_TUPLE)
		.struct_get(types::T_TUPLE, 1)
		.i32(i)
		.array_get(types::T_VALARRAY);
}

/// Push the unit `nothing` value.
fn push_nothing(w: &mut Wat) {
	w.i32(types::TAG_NOTHING).struct_new(types::T_VALUE);
}

/// Push an empty `$list`.
fn empty_list(w: &mut Wat) {
	w.i32(types::TAG_LIST);
	w.array_new_fixed(types::T_VALARRAY, 0);
	w.struct_new(types::T_LIST);
}

/// Box an i32 (pushed by `push`) as a `$int`.
fn box_i(w: &mut Wat, push: impl FnOnce(&mut Wat)) {
	w.i32(types::TAG_INT);
	push(w);
	w.i64_extend_i32_s();
	w.struct_new(types::T_INT);
}

/// Box the i32 already on top of the stack as a `$int`.
fn box_i_top(w: &mut Wat) {
	let t = w.local(ValType::I32);
	w.local_set(t);
	box_i(w, |w| {
		w.local_get(t);
	});
}

/// Box an i64 (pushed by `push`) as a `$int`.
fn box_i64(w: &mut Wat, push: impl FnOnce(&mut Wat)) {
	w.i32(types::TAG_INT);
	push(w);
	w.struct_new(types::T_INT);
}

/// Push a timer entry `$tuple(box at:i64, box kind:i32, box arg:i32)`.
fn timer_entry(w: &mut Wat, at: impl FnOnce(&mut Wat), kind: i32, arg: impl FnOnce(&mut Wat)) {
	w.i32(types::TAG_TUPLE);
	box_i64(w, at);
	box_i(w, |w| {
		w.i32(kind);
	});
	box_i(w, arg);
	w.array_new_fixed(types::T_VALARRAY, 3);
	w.struct_new(types::T_TUPLE);
}

/// Unbox the `$int`(-shaped) value on top of the stack to an i32.
fn unbox_i(w: &mut Wat) {
	w.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64();
}

/// Push `array.len` of the `$list` held in global `gl`.
fn list_len(w: &mut Wat, gl: u32) {
	w.global_get(gl)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.array_len();
}

/// Push field `field` of record `id` in the `$list` table at global `table`.
fn fld(w: &mut Wat, _g: TaskGlobals, table: u32, id: Local, field: u32) {
	w.global_get(table)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1);
	w.local_get(id).array_get(types::T_VALARRAY);
	w.ref_cast(types::T_TUPLE).struct_get(types::T_TUPLE, 1);
	w.i32(field as i32).array_get(types::T_VALARRAY);
}

/// Push field `field` of record `id` unboxed to i32.
fn fld_i(w: &mut Wat, g: TaskGlobals, table: u32, id: Local, field: u32) {
	fld(w, g, table, id, field);
	unbox_i(w);
}

/// Set field `field` of record `id` to the value pushed by `push`.
fn set_fld(w: &mut Wat, table: u32, id: Local, field: u32, push: impl FnOnce(&mut Wat)) {
	w.global_get(table)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1);
	w.local_get(id).array_get(types::T_VALARRAY);
	w.ref_cast(types::T_TUPLE).struct_get(types::T_TUPLE, 1);
	w.i32(field as i32);
	push(w);
	w.array_set(types::T_VALARRAY);
}

/// Set field `field` of record `id` to a boxed i32 pushed by `push`.
fn set_fld_i(w: &mut Wat, table: u32, id: Local, field: u32, push: impl FnOnce(&mut Wat)) {
	set_fld(w, table, id, field, |w| {
		box_i(w, push);
	});
}

/// Build a new fiber `$tuple`'s field list `[ACT=[] .. WAITERS=[]]` given the
/// owning scope and runs-scope (both i32 pushers).
fn fiber_fields(w: &mut Wat, scope_id: impl FnOnce(&mut Wat), runs_scope: impl FnOnce(&mut Wat)) {
	empty_list(w); // ACT
	box_i(w, scope_id); // SCOPE
	box_i(w, runs_scope); // RUNS_SCOPE
	box_i(w, |w| {
		w.i32(outcome::NONE);
	}); // RES_KIND
	push_nothing(w); // RES_VAL
	box_i(w, |w| {
		w.i32(wait::NONE);
	}); // WAIT_KIND
	box_i(w, |w| {
		w.i32(0);
	}); // WAIT_ARG
	box_i(w, |w| {
		w.i32(1);
	}); // ALIVE
	empty_list(w); // WAITERS
	w.array_new_fixed(types::T_VALARRAY, fiber::COUNT);
}

/// Build a new scope `$tuple`'s field list given manual flag, body fid, awaiter.
fn scope_fields(
	w: &mut Wat,
	manual: impl FnOnce(&mut Wat),
	body: impl FnOnce(&mut Wat),
	awaiter: impl FnOnce(&mut Wat),
) {
	box_i(w, manual); // MANUAL
	box_i(w, |w| {
		w.i32(0);
	}); // CANCELLED
	box_i(w, |w| {
		w.i32(0);
	}); // FINALIZED
	box_i(w, body); // BODY
	empty_list(w); // CHILDREN
	box_i(w, awaiter); // AWAITER
	box_i(w, |w| {
		w.i32(outcome::NONE);
	}); // BD_KIND
	push_nothing(w); // BD_VAL
	box_i(w, |w| {
		w.i32(0);
	}); // FAIL_SET
	push_nothing(w); // FAIL_VAL
	empty_list(w); // COMPLETED
	empty_list(w); // NEXT_WAITERS
	w.array_new_fixed(types::T_VALARRAY, scope::COUNT);
}

/// Push a fresh root-fiber `$tuple` onto the stack (for `run_task`'s seed).
fn push_fiber(w: &mut Wat, scope_id: i64, runs_scope: i64) {
	w.i32(types::TAG_TUPLE);
	fiber_fields(
		w,
		|w| {
			w.i32(scope_id as i32);
		},
		|w| {
			w.i32(runs_scope as i32);
		},
	);
	w.struct_new(types::T_TUPLE);
}

/// Push a fresh root-scope `$tuple` onto the stack (for `run_task`'s seed).
fn push_scope(w: &mut Wat, manual: i64, awaiter: i64, body: u32) {
	let _ = body;
	w.i32(types::TAG_TUPLE);
	box_i(w, |w| {
		w.i32(manual as i32);
	}); // MANUAL
	box_i(w, |w| {
		w.i32(0);
	}); // CANCELLED
	box_i(w, |w| {
		w.i32(0);
	}); // FINALIZED
	box_i(w, |w| {
		w.i32(0);
	}); // BODY = 0 (root)
	empty_list(w); // CHILDREN
	box_i(w, |w| {
		w.i32(awaiter as i32);
	}); // AWAITER
	box_i(w, |w| {
		w.i32(outcome::NONE);
	}); // BD_KIND
	push_nothing(w); // BD_VAL
	box_i(w, |w| {
		w.i32(0);
	}); // FAIL_SET
	push_nothing(w); // FAIL_VAL
	empty_list(w); // COMPLETED
	empty_list(w); // NEXT_WAITERS
	w.array_new_fixed(types::T_VALARRAY, scope::COUNT);
	w.struct_new(types::T_TUPLE);
}

/// Push a ready-deque entry `$tuple(fid, focus_kind, val)`.
fn push_ready_entry(w: &mut Wat, fid: u32, fk: i32, val: impl FnOnce(&mut Wat)) {
	w.i32(types::TAG_TUPLE);
	box_i(w, |w| {
		w.i32(fid as i32);
	});
	box_i(w, |w| {
		w.i32(fk);
	});
	val(w);
	w.array_new_fixed(types::T_VALARRAY, 3);
	w.struct_new(types::T_TUPLE);
}

/// `ready.append((fid, fk, val))` with `fid` an i32 local.
fn ready_push(
	w: &mut Wat,
	g: TaskGlobals,
	list_append: u32,
	fid: Local,
	fk: i32,
	val: impl FnOnce(&mut Wat),
) {
	w.global_get(g.ready);
	w.i32(types::TAG_TUPLE);
	box_i(w, |w| {
		w.local_get(fid);
	});
	box_i(w, |w| {
		w.i32(fk);
	});
	val(w);
	w.array_new_fixed(types::T_VALARRAY, 3);
	w.struct_new(types::T_TUPLE);
	w.call(list_append);
	w.global_set(g.ready);
}

/// `ready.append((fid, focus-of(kind), val-or-nothing))` for an outcome.
fn ready_push_outcome(
	w: &mut Wat,
	g: TaskGlobals,
	list_append: u32,
	fid: Local,
	kind: Local,
	val: Local,
	_lits: TaskLits,
) {
	w.local_get(kind).i32(outcome::OK).i32_eq();
	w.if_else(
		|w| {
			ready_push(w, g, list_append, fid, focus::OK, |w| {
				w.local_get(val);
			});
		},
		|w| {
			w.local_get(kind).i32(outcome::ERR).i32_eq();
			w.if_else(
				|w| {
					ready_push(w, g, list_append, fid, focus::ERR, |w| {
						w.local_get(val);
					});
				},
				|w| {
					// cancelled -> Ok(nothing).
					ready_push(w, g, list_append, fid, focus::OK, push_nothing);
				},
			);
		},
	);
}

/// Build an outcome `$tuple(boxed kind, val)` (for the `completed` queue).
fn mk_outcome(w: &mut Wat, kind: Local, val: Local) {
	w.i32(types::TAG_TUPLE);
	box_i(w, |w| {
		w.local_get(kind);
	});
	w.local_get(val);
	w.array_new_fixed(types::T_VALARRAY, 2);
	w.struct_new(types::T_TUPLE);
}

/// Drop the last element of the `$list` in global `gl` (rebuild via slice).
fn drop_last(w: &mut Wat, gl: u32) {
	let arr = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let out = w.local(types::valarray_ref());
	w.global_get(gl)
		.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(arr);
	w.local_get(arr).array_len().i32(1).i32_sub().local_set(n);
	w.local_get(n)
		.array_new_default(types::T_VALARRAY)
		.local_set(out);
	w.copy_loop(types::T_VALARRAY, out, None, arr, None, n);
	w.i32(types::TAG_LIST);
	w.local_get(out);
	w.struct_new(types::T_LIST);
	w.global_set(gl);
}

/// Load fiber `fid`'s activation chain into the working stack (a fresh copy).
fn load_act(w: &mut Wat, g: TaskGlobals, fid: Local) {
	let arr = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let na = w.local(types::valarray_ref());
	fld(w, g, g.fibers, fid, fiber::ACT);
	w.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(arr);
	w.local_get(arr).array_len().local_set(n);
	// na = new array(max(n, 1)); copy.
	w.local_get(n).i32_eqz();
	w.if_result(
		ValType::I32,
		|w| {
			w.i32(1);
		},
		|w| {
			w.local_get(n);
		},
	);
	w.array_new_default(types::T_VALARRAY).local_set(na);
	w.copy_loop(types::T_VALARRAY, na, None, arr, None, n);
	w.local_get(na).global_set(g.act);
	w.local_get(n).global_set(g.actlen);
}

/// Save the working stack back into fiber `fid`'s activation chain (fresh `$list`).
fn save_act(w: &mut Wat, g: TaskGlobals, fid: Local) {
	let out = w.local(types::valarray_ref());
	let src = w.local(types::valarray_ref_null());
	let len = w.local(ValType::I32);
	w.global_get(g.actlen).local_set(len);
	w.local_get(len)
		.array_new_default(types::T_VALARRAY)
		.local_set(out);
	w.global_get(g.act).local_set(src);
	w.copy_loop(types::T_VALARRAY, out, None, src, None, len);
	set_fld(w, g.fibers, fid, fiber::ACT, |w| {
		w.i32(types::TAG_LIST);
		w.local_get(out);
		w.struct_new(types::T_LIST);
	});
}

/// Write a "done" outcome to the pump output channel.
fn done_out(w: &mut Wat, g: TaskGlobals, kind: i32, val: impl FnOnce(&mut Wat)) {
	w.i32(1).global_set(g.out_kind);
	w.i32(kind).global_set(g.out_okerr);
	val(w);
	w.global_set(g.out_val);
}

/// Write a "park" with no arg to the pump output channel.
fn park_out(w: &mut Wat, g: TaskGlobals, wait_kind: i32, arg: impl FnOnce(&mut Wat)) {
	w.i32(2).global_set(g.out_kind);
	w.i32(wait_kind).global_set(g.out_okerr);
	arg(w);
	w.global_set(g.out_arg);
}

/// A Start arm that settles directly.
fn start_settle(
	w: &mut Wat,
	tk: Local,
	kind: i32,
	next: i32,
	val: impl FnOnce(&mut Wat),
	fval: Local,
	fkind: Local,
) {
	w.local_get(tk).i32(kind).i32_eq();
	w.if_(|w| {
		val(w);
		w.local_set(fval);
		w.i32(next).local_set(fkind);
		w.br("main");
	});
}

/// A Start arm for a sequential combinator (push an activation, run the inner).
#[allow(clippy::too_many_arguments)]
fn start_combinator(
	w: &mut Wat,
	tk: Local,
	kind: i32,
	akind: i32,
	tp: Local,
	has_arg: bool,
	act_push: u32,
	fval: Local,
	fkind: Local,
) {
	w.local_get(tk).i32(kind).i32_eq();
	w.if_(|w| {
		if has_arg {
			push_activation(w, akind, |w| elem(w, tp, 1), push_nothing);
		} else {
			push_activation(w, akind, push_nothing, push_nothing);
		}
		w.call(act_push).drop();
		elem(w, tp, 0);
		w.local_set(fval);
		w.i32(focus::START).local_set(fkind);
		w.br("main");
	});
}

/// After `__poll_step`: start the tail task (complete) or push a `Poll` and start
/// the sub-task (pending).
#[allow(clippy::too_many_arguments)]
fn poll_after(
	w: &mut Wat,
	pc: Local,
	ps: Local,
	pspl: Local,
	psk: Local,
	fval: Local,
	fkind: Local,
	act_push: u32,
) {
	w.local_get(ps)
		.ref_cast(types::T_TUPLE)
		.struct_get(types::T_TUPLE, 1)
		.local_set(pspl);
	w.local_get(pspl)
		.i32(0)
		.array_get(types::T_VALARRAY)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.i32_wrap_i64()
		.local_set(psk);
	w.local_get(psk).i32_eqz();
	w.if_else(
		|w| {
			elem(w, pspl, 1);
			w.local_set(fval);
			w.i32(focus::START).local_set(fkind);
			w.br("main");
		},
		|w| {
			push_activation(
				w,
				act_kind::POLL,
				|w| {
					w.local_get(pc);
				},
				|w| elem(w, pspl, 2),
			);
			w.call(act_push).drop();
			elem(w, pspl, 1);
			w.local_set(fval);
			w.i32(focus::START).local_set(fkind);
			w.br("main");
		},
	);
}

/// Pop the top activation off the working stack into `(a, akind, apl)`.
fn pop_activation(w: &mut Wat, g: TaskGlobals, a: Local, akind: Local, apl: Local) {
	w.global_get(g.actlen).i32(1).i32_sub().global_set(g.actlen);
	w.global_get(g.act)
		.global_get(g.actlen)
		.array_get(types::T_VALARRAY)
		.local_set(a);
	w.local_get(a)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 1)
		.local_set(akind);
	w.local_get(a)
		.ref_cast(types::T_VARIANT)
		.struct_get(types::T_VARIANT, 3)
		.local_set(apl);
}

/// Push an activation `$variant` `{vtag: kind, payload: [x, y]}` (name unused).
fn push_activation(w: &mut Wat, kind: i32, x: impl FnOnce(&mut Wat), y: impl FnOnce(&mut Wat)) {
	w.i32(types::TAG_VARIANT);
	w.i32(kind);
	w.ref_null(types::T_VALUE);
	x(w);
	y(w);
	w.array_new_fixed(types::T_VALARRAY, 2);
	w.struct_new(types::T_VARIANT);
}

/// Push a 3-tuple `(box kind, x, y)` — the `__poll_step` result shape.
fn push_tuple3(w: &mut Wat, kind: i64, x: impl FnOnce(&mut Wat), y: impl FnOnce(&mut Wat)) {
	w.i32(types::TAG_TUPLE);
	w.i32(types::TAG_INT).i64(kind).struct_new(types::T_INT);
	x(w);
	y(w);
	w.array_new_fixed(types::T_VALARRAY, 3);
	w.struct_new(types::T_TUPLE);
}

/// Push a `result`/`option` `$variant` `{vtag: tag, name, payload: [<value>]}`.
fn push_result(w: &mut Wat, tag: u32, name: (u32, u32), val: impl FnOnce(&mut Wat)) {
	w.i32(types::TAG_VARIANT);
	w.i32(tag as i32);
	str_lit(w, name);
	val(w);
	w.array_new_fixed(types::T_VALARRAY, 1);
	w.struct_new(types::T_VARIANT);
}

/// Call a 1-arg closure: leaves its result on the stack.
fn call1(w: &mut Wat, clo: impl Fn(&mut Wat), arg: impl FnOnce(&mut Wat), arity1: u32) {
	clo(w);
	w.ref_cast(types::T_CLOSURE);
	arg(w);
	clo(w);
	w.ref_cast(types::T_CLOSURE).struct_get(types::T_CLOSURE, 1);
	w.call_indirect(arity1);
}

/// Push a fresh `$str` for an interned data-segment literal `(off, len)`.
fn str_lit(w: &mut Wat, (off, len): (u32, u32)) {
	w.i32(types::TAG_STR);
	w.i32(off as i32);
	w.i32(len as i32);
	w.array_new_data(types::T_BYTES, 0);
	w.struct_new(types::T_STR);
}

/// Push a `$tuple(box action, val)` — the `__drain_next` result shape.
fn action_tuple(w: &mut Wat, action: i64, val: impl FnOnce(&mut Wat)) {
	w.i32(types::TAG_TUPLE);
	w.i32(types::TAG_INT).i64(action).struct_new(types::T_INT);
	val(w);
	w.array_new_fixed(types::T_VALARRAY, 2);
	w.struct_new(types::T_TUPLE);
}

/// Push `option.some(<val>)`.
fn push_some(w: &mut Wat, lits: TaskLits, val: impl FnOnce(&mut Wat)) {
	push_result(w, lits.some_tag, lits.some_name, val);
}

/// Push `option.none`.
fn push_none(w: &mut Wat, lits: TaskLits) {
	w.i32(types::TAG_VARIANT);
	w.i32(lits.none_tag as i32);
	str_lit(w, lits.none_name);
	w.array_new_fixed(types::T_VALARRAY, 0);
	w.struct_new(types::T_VARIANT);
}

/// Push the `result` a settled child outcome yields: `ok v` / `err e` (cancelled
/// → `ok ()`). `oc` is a `$tuple(boxed kind, val)`.
fn push_settled(w: &mut Wat, lits: TaskLits, oc: Local) {
	let k = w.local(ValType::I32);
	w.local_get(oc)
		.ref_cast(types::T_TUPLE)
		.struct_get(types::T_TUPLE, 1)
		.i32(0)
		.array_get(types::T_VALARRAY);
	unbox_i(w);
	w.local_set(k);
	w.local_get(k).i32(outcome::OK).i32_eq();
	w.if_result(
		types::value_ref(),
		|w| {
			push_result(w, lits.ok_tag, lits.ok_name, |w| {
				w.local_get(oc)
					.ref_cast(types::T_TUPLE)
					.struct_get(types::T_TUPLE, 1)
					.i32(1)
					.array_get(types::T_VALARRAY);
			});
		},
		|w| {
			w.local_get(k).i32(outcome::ERR).i32_eq();
			w.if_result(
				types::value_ref(),
				|w| {
					push_result(w, lits.err_tag, lits.err_name, |w| {
						w.local_get(oc)
							.ref_cast(types::T_TUPLE)
							.struct_get(types::T_TUPLE, 1)
							.i32(1)
							.array_get(types::T_VALARRAY);
					});
				},
				|w| {
					// cancelled -> ok ().
					push_result(w, lits.ok_tag, lits.ok_name, push_nothing);
				},
			);
		},
	);
}

/// Drop the first element of the `$valarray` `arr` (length `n`) and wrap the rest
/// as a fresh `$list`.
fn drop_first_list(w: &mut Wat, arr: Local, n: Local) {
	let out = w.local(types::valarray_ref());
	let one = w.local(ValType::I32);
	let len = w.local(ValType::I32);
	w.local_get(n).i32(1).i32_sub().local_set(len);
	w.local_get(len)
		.array_new_default(types::T_VALARRAY)
		.local_set(out);
	// out[0..n-1] = arr[1..n] (drop first; see `Wat::copy_loop`).
	w.i32(1).local_set(one);
	w.copy_loop(types::T_VALARRAY, out, None, arr, Some(one), len);
	w.i32(types::TAG_LIST);
	w.local_get(out);
	w.struct_new(types::T_LIST);
}

/// Push i32 1 if every child of scope `sid` has settled (none alive), else 0.
fn all_children_done(w: &mut Wat, g: TaskGlobals, sid: Local) {
	let children = w.local(types::valarray_ref());
	let n = w.local(ValType::I32);
	let i = w.local(ValType::I32);
	let res = w.local(ValType::I32);
	fld(w, g, g.scopes, sid, scope::CHILDREN);
	w.ref_cast(types::T_LIST)
		.struct_get(types::T_LIST, 1)
		.local_set(children);
	w.local_get(children).array_len().local_set(n);
	w.i32(0).local_set(i);
	w.i32(1).local_set(res);
	w.block("brk", |w| {
		w.loop_("lp", |w| {
			w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
			w.local_get(children)
				.local_get(i)
				.array_get(types::T_VALARRAY);
			let c = w.local(ValType::I32);
			unbox_i(w);
			w.local_set(c);
			fld_i(w, g, g.fibers, c, fiber::ALIVE);
			w.if_(|w| {
				w.i32(0).local_set(res);
				w.br("brk");
			});
			w.local_get(i).i32(1).i32_add().local_set(i);
			w.br("lp");
		});
	});
	w.local_get(res);
}
