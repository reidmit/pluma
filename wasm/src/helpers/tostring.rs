// `to-string` rendering: decimal int formatting + the recursive `__tostring`
// (`vm::Value`'s `Display` in wasm).

use wasm_encoder::{Function, ValType};

use crate::helpers::wat::{Local, Wat};
use crate::runtime::ToStringLits;
use crate::types;

/// Build `__int_str(boxed-int) -> str`: decimal formatting of an i64. Mirrors
/// `vm::Value`'s `Display` for ints (`-` sign, no leading zeros, "0" for zero).
pub(crate) fn build_int_str_fn() -> Function {
	let bv = types::T_BYTES;
	let mut w = Wat::new(1);
	let v = w.param(0); // boxed $int
	let n = w.local(ValType::I64); // i64 value
	let neg = w.local(ValType::I32);
	let m = w.local(ValType::I64); // abs value
	let len = w.local(ValType::I32);
	let total = w.local(ValType::I32);
	let buf = w.local(types::bytes_ref());
	let i = w.local(ValType::I32);
	let q = w.local(ValType::I64);

	// Wrap `buf` in a `$str` and return.
	let mk_str = |w: &mut Wat| {
		w.i32(types::TAG_STR)
			.local_get(buf)
			.struct_new(types::T_STR)
			.ret();
	};

	w.local_get(v)
		.ref_cast(types::T_INT)
		.struct_get(types::T_INT, 1)
		.local_set(n);
	// n == 0 -> "0".
	w.local_get(n).i64_eqz();
	w.if_(|w| {
		w.i32(1).array_new_default(bv).local_set(buf);
		w.local_get(buf).i32(0).i32(48).array_set(bv); // '0'
		mk_str(w);
	});
	// neg = n < 0.
	w.local_get(n).i64(0).i64_lt_s().local_set(neg);
	// m = n; if neg { m = 0 - n }.
	w.local_get(n).local_set(m);
	w.local_get(neg);
	w.if_(|w| {
		w.i64(0).local_get(n).i64_sub().local_set(m);
	});
	// count digits: len=0; q=m; do { len++; q/=10 } while q!=0.
	w.i32(0).local_set(len);
	w.local_get(m).local_set(q);
	w.loop_("count", |w| {
		w.local_get(len).i32(1).i32_add().local_set(len);
		w.local_get(q).i64(10).i64_div_s().local_set(q);
		w.local_get(q).i64_eqz().i32_eqz().br_if("count"); // q != 0 -> loop
	});
	// total = len + neg.
	w.local_get(len).local_get(neg).i32_add().local_set(total);
	w.local_get(total).array_new_default(bv).local_set(buf);
	// fill from end: i = total-1; q = m; do { buf[i]=48+(q%10); q/=10; i-- } while q!=0.
	w.local_get(total).i32(1).i32_sub().local_set(i);
	w.local_get(m).local_set(q);
	w.loop_("fill", |w| {
		w.local_get(buf).local_get(i);
		w.i32(48)
			.local_get(q)
			.i64(10)
			.i64_rem_s()
			.i32_wrap_i64()
			.i32_add();
		w.array_set(bv);
		w.local_get(q).i64(10).i64_div_s().local_set(q);
		w.local_get(i).i32(1).i32_sub().local_set(i);
		w.local_get(q).i64_eqz().i32_eqz().br_if("fill"); // q != 0 -> loop
	});
	// if neg { buf[0] = '-' }.
	w.local_get(neg);
	w.if_(|w| {
		w.local_get(buf).i32(0).i32(45).array_set(bv); // '-'
	});
	mk_str(&mut w);
	w.finish()
}

/// Build `__tostring(value) -> str` — `vm::Value`'s `Display` in wasm. Scalars +
/// string (identity) + int (`__int_str`) + float (host `float_to_str`); compounds
/// (tuple/list/record/variant) are formatted recursively, folding byte arrays with
/// `__bytesconcat`. `self_idx` is `__tostring`'s own index (for the recursion).
pub(crate) fn build_tostring_fn(
	self_idx: u32,
	int_str: u32,
	bc: u32,
	float_to_str: u32,
	dict_entries: u32,
	lits: ToStringLits,
) -> Function {
	let bv = types::T_BYTES;
	let mut w = Wat::new(1);
	let v = w.param(0);
	let ta = w.local(ValType::I32);
	let acc = w.local(types::bytes_ref()); // $bytes accumulator
	let i = w.local(ValType::I32);
	let n = w.local(ValType::I32);
	let arr = w.local(types::valarray_ref()); // tuple/list elems, variant payload, record values
	let names = w.local(types::valarray_ref()); // record names
	let buf = w.local(types::bytes_ref()); // float scratch; also bytes-arm source/dst
	let len = w.local(ValType::I32); // float len; also bytes-arm write position
	let byte = w.local(ValType::I32); // bytes-arm current byte
	let nib = w.local(ValType::I32); // bytes-arm hex nibble scratch

	// Push a `$bytes` array for a data-segment literal.
	let lit_bytes = |w: &mut Wat, (off, len): (u32, u32)| {
		w.i32(off as i32).i32(len as i32).array_new_data(bv, 0);
	};
	// `acc = __bytesconcat(acc, <literal>)`.
	let cat_lit = |w: &mut Wat, lit: (u32, u32)| {
		w.local_get(acc);
		lit_bytes(w, lit);
		w.call(bc).local_set(acc);
	};
	// `acc` -> `$str`; return.
	let wrap = |w: &mut Wat| {
		w.i32(types::TAG_STR)
			.local_get(acc)
			.struct_new(types::T_STR)
			.ret();
	};
	// Return a fresh `$str` of a data-segment literal directly.
	let mk_lit = |w: &mut Wat, lit: (u32, u32)| {
		w.i32(types::TAG_STR);
		lit_bytes(w, lit);
		w.struct_new(types::T_STR).ret();
	};
	// `acc = __bytesconcat(acc, strbytes(__tostring(arr[i])))`.
	let cat_tostring_of = |w: &mut Wat, a: Local| {
		w.local_get(acc);
		w.local_get(a).local_get(i).array_get(types::T_VALARRAY);
		w.call(self_idx)
			.ref_cast(types::T_STR)
			.struct_get(types::T_STR, 1); // -> $str bytes
		w.call(bc).local_set(acc);
	};
	// Append the constant byte `code` to acc[len], then bump len.
	let put = |w: &mut Wat, code: i32| {
		w.local_get(acc).local_get(len).i32(code).array_set(bv);
		w.local_get(len).i32(1).i32_add().local_set(len);
	};
	// Append one lowercase hex digit for the nibble of `byte` at `shift`.
	let put_hex = |w: &mut Wat, shift: i32| {
		w.local_get(byte);
		if shift != 0 {
			w.i32(shift).i32_shr_u();
		}
		w.i32(0xf).i32_and().local_set(nib);
		w.local_get(acc).local_get(len);
		// digit = nib < 10 ? '0'+nib : 'a'-10+nib  (0x61-10 = 0x57).
		w.local_get(nib).i32(10).i32_lt_s();
		w.if_result(
			ValType::I32,
			|w| {
				w.local_get(nib).i32(0x30).i32_add();
			},
			|w| {
				w.local_get(nib).i32(0x57).i32_add();
			},
		);
		w.array_set(bv);
		w.local_get(len).i32(1).i32_add().local_set(len);
	};

	w.local_get(v).struct_get(types::T_VALUE, 0).local_set(ta);

	// STR -> identity.
	w.local_get(ta).i32(types::TAG_STR).i32_eq();
	w.if_(|w| {
		w.local_get(v).ret();
	});
	// INT -> __int_str.
	w.local_get(ta).i32(types::TAG_INT).i32_eq();
	w.if_(|w| {
		w.local_get(v).call(int_str).ret();
	});
	// NOTHING -> "()".
	w.local_get(ta).i32(types::TAG_NOTHING).i32_eq();
	w.if_(|w| mk_lit(w, lits.unit));
	// BOOL -> "true"/"false".
	w.local_get(ta).i32(types::TAG_BOOL).i32_eq();
	w.if_(|w| {
		w.local_get(v)
			.ref_cast(types::T_BOOL)
			.struct_get(types::T_BOOL, 1);
		w.if_else(|w| mk_lit(w, lits.tru), |w| mk_lit(w, lits.fals));
	});
	// FLOAT -> host float_to_str into a scratch $bytes, trim to length.
	w.local_get(ta).i32(types::TAG_FLOAT).i32_eq();
	w.if_(|w| {
		w.i32(32).array_new_default(bv).local_set(buf);
		w.local_get(v)
			.ref_cast(types::T_FLOAT)
			.struct_get(types::T_FLOAT, 1);
		w.local_get(buf).call(float_to_str).local_set(len); // (f64, buf) -> len
		w.local_get(len).array_new_default(bv).local_set(acc);
		w.local_get(acc)
			.i32(0)
			.local_get(buf)
			.i32(0)
			.local_get(len)
			.array_copy(bv, bv);
		wrap(w);
	});

	// BYTES -> single-quoted literal form: printable ASCII inline, `'` and `\`
	// backslash-escaped, everything else as `\xNN` (lowercase). Matches
	// `Value::Display` so wasm `to-string` agrees with the VM. Writes into a
	// worst-case (4 bytes/input + 2 quotes) buffer, then trims — no concat.
	// buf=source/dst, acc=output buffer, n=source len, len=write position.
	w.local_get(ta).i32(types::TAG_BYTES).i32_eq();
	w.if_(|w| {
		// buf = source bytes; n = its length.
		w.local_get(v)
			.ref_cast(types::T_STR)
			.struct_get(types::T_STR, 1)
			.local_set(buf);
		w.local_get(buf).array_len().local_set(n);
		// acc = new $bytes[n*4 + 2]; len (write pos) = 0.
		w.local_get(n)
			.i32(4)
			.i32_mul()
			.i32(2)
			.i32_add()
			.array_new_default(bv)
			.local_set(acc);
		w.i32(0).local_set(len);
		put(w, 0x27); // opening '
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
				// byte = source[i] (unsigned).
				w.local_get(buf)
					.local_get(i)
					.array_get_u(bv)
					.local_set(byte);
				w.local_get(byte).i32(0x5c).i32_eq();
				w.if_else(
					|w| {
						put(w, 0x5c);
						put(w, 0x5c);
					},
					|w| {
						w.local_get(byte).i32(0x27).i32_eq();
						w.if_else(
							|w| {
								put(w, 0x5c);
								put(w, 0x27);
							},
							|w| {
								w.local_get(byte).i32(0x20).i32_ge_s();
								w.local_get(byte).i32(0x7e).i32_le_s();
								w.i32_and();
								w.if_else(
									|w| {
										// printable: copy the byte verbatim.
										w.local_get(acc)
											.local_get(len)
											.local_get(byte)
											.array_set(bv);
										w.local_get(len).i32(1).i32_add().local_set(len);
									},
									|w| {
										put(w, 0x5c); // '\'
										put(w, 0x78); // 'x'
										put_hex(w, 4);
										put_hex(w, 0);
									},
								);
							},
						);
					},
				);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		put(w, 0x27); // closing '
		// Trim acc[0..len] into a tight $bytes (buf), then wrap as $str.
		w.local_get(len).array_new_default(bv).local_set(buf);
		w.local_get(buf)
			.i32(0)
			.local_get(acc)
			.i32(0)
			.local_get(len)
			.array_copy(bv, bv);
		w.local_get(buf).local_set(acc);
		wrap(w);
	});

	// Element loop shared by TUPLE/LIST/RECORD: iterate arr[0..n] appending
	// `__tostring(elem)` with `, ` separators. `open`/`close` wrap the delimiters.
	// The caller sets `n` to the logical element count first (field 2 for a
	// `$list`, `array.len` for the exact-sized tuple/record arrays).
	let elems_loop = |w: &mut Wat, a: Local, open: (u32, u32), close: (u32, u32), record: bool| {
		// acc = open.
		lit_bytes(w, open);
		w.local_set(acc);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
				// separator before all but the first.
				w.local_get(i).i32(0).i32_gt_s();
				w.if_(|w| cat_lit(w, lits.comma_sp));
				if record {
					// "name: value": names[i] is a raw $str; values in arr.
					w.local_get(acc);
					w.local_get(names).local_get(i).array_get(types::T_VALARRAY);
					w.ref_cast(types::T_STR).struct_get(types::T_STR, 1);
					w.call(bc).local_set(acc);
					cat_lit(w, lits.colon_sp);
				}
				cat_tostring_of(w, a);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		cat_lit(w, close);
		wrap(w);
	};

	// TUPLE -> "(e, ...)".
	w.local_get(ta).i32(types::TAG_TUPLE).i32_eq();
	w.if_(|w| {
		w.local_get(v)
			.ref_cast(types::T_TUPLE)
			.struct_get(types::T_TUPLE, 1)
			.local_set(arr);
		w.local_get(arr).array_len().local_set(n);
		elems_loop(w, arr, lits.lparen, lits.rparen, false);
	});
	// LIST -> "[e, ...]".
	w.local_get(ta).i32(types::TAG_LIST).i32_eq();
	w.if_(|w| {
		w.local_get(v)
			.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(arr);
		// the logical length (field 2), not array.len (capacity).
		w.local_get(v)
			.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 2)
			.local_set(n);
		elems_loop(w, arr, lits.lbrack, lits.rbrack, false);
	});
	// RECORD -> "{k: v, ...}" (name-sorted; names raw, values via __tostring).
	w.local_get(ta).i32(types::TAG_RECORD).i32_eq();
	w.if_(|w| {
		w.local_get(v)
			.ref_cast(types::T_RECORD)
			.struct_get(types::T_RECORD, 1)
			.local_set(names);
		w.local_get(v)
			.ref_cast(types::T_RECORD)
			.struct_get(types::T_RECORD, 2)
			.local_set(arr);
		w.local_get(arr).array_len().local_set(n);
		elems_loop(w, arr, lits.lbrace, lits.rbrace, true);
	});
	// VARIANT -> "enum.variant" then ` arg` per payload element.
	w.local_get(ta).i32(types::TAG_VARIANT).i32_eq();
	w.if_(|w| {
		// acc = bytes-of(name).
		w.local_get(v)
			.ref_cast(types::T_VARIANT)
			.struct_get(types::T_VARIANT, 2);
		w.ref_cast(types::T_STR)
			.struct_get(types::T_STR, 1)
			.local_set(acc);
		w.local_get(v)
			.ref_cast(types::T_VARIANT)
			.struct_get(types::T_VARIANT, 3)
			.local_set(arr);
		w.local_get(arr).array_len().local_set(n);
		w.i32(0).local_set(i);
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
				cat_lit(w, lits.space);
				cat_tostring_of(w, arr);
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		wrap(w);
	});

	// REF -> "ref <inner>" (matches `vm::Value`'s Display).
	w.local_get(ta).i32(types::TAG_REF).i32_eq();
	w.if_(|w| {
		// acc = bytes-of "ref ".
		lit_bytes(w, lits.ref_pfx);
		w.local_set(acc);
		// acc = bytesconcat(acc, strbytes(tostring(cell))).
		w.local_get(acc);
		w.local_get(v)
			.ref_cast(types::T_REF)
			.struct_get(types::T_REF, 1);
		w.call(self_idx)
			.ref_cast(types::T_STR)
			.struct_get(types::T_STR, 1); // -> $str bytes
		w.call(bc).local_set(acc);
		wrap(w);
	});

	// DICT -> "{k: v, ...}" (insertion order; each entry a `$tuple`). Mirrors
	// `vm::Value`'s Dict Display. `__dict_entries` materializes the seq-ordered
	// `(key, value)` list; `arr`/`n` are its backing array + length.
	w.local_get(ta).i32(types::TAG_DICT).i32_eq();
	w.if_(|w| {
		w.local_get(v)
			.call(dict_entries)
			.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 1)
			.local_set(arr);
		w.local_get(v)
			.call(dict_entries)
			.ref_cast(types::T_LIST)
			.struct_get(types::T_LIST, 2)
			.local_set(n);
		// acc = "{"  (set, not concat — acc is not yet initialized here).
		lit_bytes(w, lits.lbrace);
		w.local_set(acc);
		w.i32(0).local_set(i);
		// key/value of entry i, formatted via __tostring and folded into acc.
		let entry_elem = |w: &mut Wat, field: i32| {
			w.local_get(acc);
			w.local_get(arr).local_get(i).array_get(types::T_VALARRAY);
			w.ref_cast(types::T_TUPLE).struct_get(types::T_TUPLE, 1);
			w.i32(field).array_get(types::T_VALARRAY);
			w.call(self_idx)
				.ref_cast(types::T_STR)
				.struct_get(types::T_STR, 1); // -> $str bytes
			w.call(bc).local_set(acc);
		};
		w.block("brk", |w| {
			w.loop_("lp", |w| {
				w.local_get(i).local_get(n).i32_ge_s().br_if("brk");
				// separator before all but the first.
				w.local_get(i).i32(0).i32_gt_s();
				w.if_(|w| cat_lit(w, lits.comma_sp));
				entry_elem(w, 0); // key
				cat_lit(w, lits.colon_sp);
				entry_elem(w, 1); // value
				w.local_get(i).i32(1).i32_add().local_set(i);
				w.br("lp");
			});
		});
		cat_lit(w, lits.rbrace);
		wrap(w);
	});

	// Unreachable: every value tag is handled above.
	w.unreachable();
	w.finish()
}
