// `to-string` rendering: decimal int formatting + the recursive `__tostring`
// (`vm::Value`'s `Display` in wasm).

use wasm_encoder::*;

use crate::runtime::ToStringLits;
use crate::types;

/// Build `__int_str(boxed-int) -> str`: decimal formatting of an i64. Mirrors
/// `vm::Value`'s `Display` for ints (`-` sign, no leading zeros, "0" for zero).
pub(crate) fn build_int_str_fn() -> Function {
	use Instruction as I;
	const V: u32 = 0; // boxed $int param
	const N: u32 = 1; // i64 value
	const NEG: u32 = 2;
	const M: u32 = 3; // abs value
	const LEN: u32 = 4;
	const TOTAL: u32 = 5;
	const BUF: u32 = 6;
	const I_: u32 = 7;
	const Q: u32 = 8;
	let empty = wasm_encoder::BlockType::Empty;
	let bv = types::T_BYTES;
	let mk_str = |b: &mut Vec<Instruction>| {
		// wrap BUF in a $str and return.
		b.push(I::I32Const(types::TAG_STR));
		b.push(I::LocalGet(BUF));
		b.push(I::StructNew(types::T_STR));
		b.push(I::Return);
	};
	let locals = vec![
		ValType::I64,
		ValType::I32,
		ValType::I64,
		ValType::I32,
		ValType::I32,
		types::bytes_ref(),
		ValType::I32,
		ValType::I64,
	];
	let mut b: Vec<Instruction> = Vec::new();
	b.push(I::LocalGet(V));
	b.push(I::RefCastNonNull(HeapType::Concrete(types::T_INT)));
	b.push(I::StructGet {
		struct_type_index: types::T_INT,
		field_index: 1,
	});
	b.push(I::LocalSet(N));
	// n == 0 -> "0"
	b.push(I::LocalGet(N));
	b.push(I::I64Eqz);
	b.push(I::If(empty));
	b.push(I::I32Const(1));
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(BUF));
	b.push(I::LocalGet(BUF));
	b.push(I::I32Const(0));
	b.push(I::I32Const(48)); // '0'
	b.push(I::ArraySet(bv));
	mk_str(&mut b);
	b.push(I::End);
	// neg = n < 0
	b.push(I::LocalGet(N));
	b.push(I::I64Const(0));
	b.push(I::I64LtS);
	b.push(I::LocalSet(NEG));
	// m = n; if neg { m = 0 - n }
	b.push(I::LocalGet(N));
	b.push(I::LocalSet(M));
	b.push(I::LocalGet(NEG));
	b.push(I::If(empty));
	b.push(I::I64Const(0));
	b.push(I::LocalGet(N));
	b.push(I::I64Sub);
	b.push(I::LocalSet(M));
	b.push(I::End);
	// count digits: len=0; q=m; do { len++; q/=10 } while q!=0
	b.push(I::I32Const(0));
	b.push(I::LocalSet(LEN));
	b.push(I::LocalGet(M));
	b.push(I::LocalSet(Q));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(LEN));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(LEN));
	b.push(I::LocalGet(Q));
	b.push(I::I64Const(10));
	b.push(I::I64DivS);
	b.push(I::LocalSet(Q));
	b.push(I::LocalGet(Q));
	b.push(I::I64Eqz);
	b.push(I::I32Eqz);
	b.push(I::BrIf(0)); // q != 0 -> loop
	b.push(I::End);
	// total = len + neg
	b.push(I::LocalGet(LEN));
	b.push(I::LocalGet(NEG));
	b.push(I::I32Add);
	b.push(I::LocalSet(TOTAL));
	b.push(I::LocalGet(TOTAL));
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(BUF));
	// fill from end: i = total-1; q = m; do { buf[i]=48+(q%10); q/=10; i-- } while q!=0
	b.push(I::LocalGet(TOTAL));
	b.push(I::I32Const(1));
	b.push(I::I32Sub);
	b.push(I::LocalSet(I_));
	b.push(I::LocalGet(M));
	b.push(I::LocalSet(Q));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(BUF));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(48));
	b.push(I::LocalGet(Q));
	b.push(I::I64Const(10));
	b.push(I::I64RemS);
	b.push(I::I32WrapI64);
	b.push(I::I32Add);
	b.push(I::ArraySet(bv));
	b.push(I::LocalGet(Q));
	b.push(I::I64Const(10));
	b.push(I::I64DivS);
	b.push(I::LocalSet(Q));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Sub);
	b.push(I::LocalSet(I_));
	b.push(I::LocalGet(Q));
	b.push(I::I64Eqz);
	b.push(I::I32Eqz);
	b.push(I::BrIf(0)); // q != 0 -> loop
	b.push(I::End);
	// if neg { buf[0] = '-' }
	b.push(I::LocalGet(NEG));
	b.push(I::If(empty));
	b.push(I::LocalGet(BUF));
	b.push(I::I32Const(0));
	b.push(I::I32Const(45)); // '-'
	b.push(I::ArraySet(bv));
	b.push(I::End);
	mk_str(&mut b);
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
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
	lits: ToStringLits,
) -> Function {
	use Instruction as I;
	const V: u32 = 0;
	const TA: u32 = 1;
	const ACC: u32 = 2; // $bytes accumulator
	const I_: u32 = 3;
	const N: u32 = 4;
	const ARR: u32 = 5; // $valarray (tuple/list elems, variant payload, record values)
	const NAMES: u32 = 6; // $valarray (record names)
	const BUF: u32 = 7; // $bytes (float scratch; also bytes-arm source/dst)
	const LEN: u32 = 8; // i32 (float len; also bytes-arm write position)
	const BYTE: u32 = 9; // i32 (bytes-arm current byte)
	const NIB: u32 = 10; // i32 (bytes-arm hex nibble scratch)
	let empty = wasm_encoder::BlockType::Empty;
	let i32res = wasm_encoder::BlockType::Result(ValType::I32);
	let bv = types::T_BYTES;
	let cast = |t| I::RefCastNonNull(HeapType::Concrete(t));
	// Push a `$bytes` array for a data-segment literal.
	let lit_bytes = |b: &mut Vec<Instruction>, (off, len): (u32, u32)| {
		b.push(I::I32Const(off as i32));
		b.push(I::I32Const(len as i32));
		b.push(I::ArrayNewData {
			array_type_index: bv,
			array_data_index: 0,
		});
	};
	// `ACC = __bytesconcat(ACC, <literal>)`.
	let cat_lit = |b: &mut Vec<Instruction>, lit: (u32, u32)| {
		b.push(I::LocalGet(ACC));
		lit_bytes(b, lit);
		b.push(I::Call(bc));
		b.push(I::LocalSet(ACC));
	};
	let wrap = |b: &mut Vec<Instruction>| {
		// ACC -> $str ; return
		b.push(I::I32Const(types::TAG_STR));
		b.push(I::LocalGet(ACC));
		b.push(I::StructNew(types::T_STR));
		b.push(I::Return);
	};
	let mk_lit = |b: &mut Vec<Instruction>, lit: (u32, u32)| {
		b.push(I::I32Const(types::TAG_STR));
		lit_bytes(b, lit);
		b.push(I::StructNew(types::T_STR));
		b.push(I::Return);
	};
	// `ACC = __bytesconcat(ACC, bytes-of-str(s))` where `s` (a $str value) is from
	// applying `__tostring` to element `ARR[I_]` (or a raw $str for record names).
	// Helper emitting: ACC = bytesconcat(ACC, strbytes(tostring(ARR[idx_field])))
	let cat_tostring_of = |b: &mut Vec<Instruction>, arr: u32| {
		b.push(I::LocalGet(ACC));
		b.push(I::LocalGet(arr));
		b.push(I::LocalGet(I_));
		b.push(I::ArrayGet(types::T_VALARRAY));
		b.push(I::Call(self_idx)); // -> $str
		b.push(cast(types::T_STR));
		b.push(I::StructGet {
			struct_type_index: types::T_STR,
			field_index: 1,
		});
		b.push(I::Call(bc));
		b.push(I::LocalSet(ACC));
	};

	let locals = vec![
		ValType::I32,          // TA
		types::bytes_ref(),    // ACC
		ValType::I32,          // I_
		ValType::I32,          // N
		types::valarray_ref(), // ARR
		types::valarray_ref(), // NAMES
		types::bytes_ref(),    // BUF
		ValType::I32,          // LEN
		ValType::I32,          // BYTE
		ValType::I32,          // NIB
	];
	let mut b: Vec<Instruction> = Vec::new();
	b.push(I::LocalGet(V));
	b.push(I::StructGet {
		struct_type_index: types::T_VALUE,
		field_index: 0,
	});
	b.push(I::LocalSet(TA));

	// Scalar arm helper: if TA == tag { body }.
	let arm = |b: &mut Vec<Instruction>, tag: i32| {
		b.push(I::LocalGet(TA));
		b.push(I::I32Const(tag));
		b.push(I::I32Eq);
		b.push(I::If(empty));
	};

	// STR -> identity.
	arm(&mut b, types::TAG_STR);
	b.push(I::LocalGet(V));
	b.push(I::Return);
	b.push(I::End);
	// INT -> __int_str.
	arm(&mut b, types::TAG_INT);
	b.push(I::LocalGet(V));
	b.push(I::Call(int_str));
	b.push(I::Return);
	b.push(I::End);
	// NOTHING -> "()".
	arm(&mut b, types::TAG_NOTHING);
	mk_lit(&mut b, lits.unit);
	b.push(I::End);
	// BOOL -> "true"/"false".
	arm(&mut b, types::TAG_BOOL);
	b.push(I::LocalGet(V));
	b.push(cast(types::T_BOOL));
	b.push(I::StructGet {
		struct_type_index: types::T_BOOL,
		field_index: 1,
	});
	b.push(I::If(empty));
	mk_lit(&mut b, lits.tru);
	b.push(I::Else);
	mk_lit(&mut b, lits.fals);
	b.push(I::End);
	b.push(I::End);
	// FLOAT -> host float_to_str into a scratch $bytes, trim to length.
	arm(&mut b, types::TAG_FLOAT);
	b.push(I::I32Const(32));
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(BUF));
	b.push(I::LocalGet(V));
	b.push(cast(types::T_FLOAT));
	b.push(I::StructGet {
		struct_type_index: types::T_FLOAT,
		field_index: 1,
	});
	b.push(I::LocalGet(BUF));
	b.push(I::Call(float_to_str)); // (f64, buf) -> len
	b.push(I::LocalSet(LEN));
	b.push(I::LocalGet(LEN));
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(ACC));
	b.push(I::LocalGet(ACC));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(BUF));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(LEN));
	b.push(I::ArrayCopy {
		array_type_index_dst: bv,
		array_type_index_src: bv,
	});
	wrap(&mut b);
	b.push(I::End);

	// BYTES -> single-quoted literal form: printable ASCII inline, `'` and
	// `\` backslash-escaped, everything else as `\xNN` (lowercase). Matches
	// `Value::Display` so wasm `to-string` agrees with the VM. Writes into a
	// worst-case (4 bytes/input + 2 quotes) buffer, then trims — no concat.
	// BUF=source/dst, ACC=output buffer, N=source len, LEN=write position.
	// Append the constant byte `code` to ACC[LEN], then bump LEN.
	let put = |b: &mut Vec<Instruction>, code: i32| {
		b.push(I::LocalGet(ACC));
		b.push(I::LocalGet(LEN));
		b.push(I::I32Const(code));
		b.push(I::ArraySet(bv));
		b.push(I::LocalGet(LEN));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(LEN));
	};
	// Append one lowercase hex digit for the nibble of BYTE at `shift`.
	let put_hex = |b: &mut Vec<Instruction>, shift: i32| {
		b.push(I::LocalGet(BYTE));
		if shift != 0 {
			b.push(I::I32Const(shift));
			b.push(I::I32ShrU);
		}
		b.push(I::I32Const(0xf));
		b.push(I::I32And);
		b.push(I::LocalSet(NIB));
		b.push(I::LocalGet(ACC));
		b.push(I::LocalGet(LEN));
		// digit = NIB < 10 ? '0'+NIB : 'a'-10+NIB  (0x61-10 = 0x57)
		b.push(I::LocalGet(NIB));
		b.push(I::I32Const(10));
		b.push(I::I32LtS);
		b.push(I::If(i32res));
		b.push(I::LocalGet(NIB));
		b.push(I::I32Const(0x30));
		b.push(I::I32Add);
		b.push(I::Else);
		b.push(I::LocalGet(NIB));
		b.push(I::I32Const(0x57));
		b.push(I::I32Add);
		b.push(I::End);
		b.push(I::ArraySet(bv));
		b.push(I::LocalGet(LEN));
		b.push(I::I32Const(1));
		b.push(I::I32Add);
		b.push(I::LocalSet(LEN));
	};
	arm(&mut b, types::TAG_BYTES);
	// BUF = source bytes; N = its length.
	b.push(I::LocalGet(V));
	b.push(cast(types::T_STR));
	b.push(I::StructGet {
		struct_type_index: types::T_STR,
		field_index: 1,
	});
	b.push(I::LocalSet(BUF));
	b.push(I::LocalGet(BUF));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	// ACC = new $bytes[N*4 + 2]; LEN (write pos) = 0.
	b.push(I::LocalGet(N));
	b.push(I::I32Const(4));
	b.push(I::I32Mul);
	b.push(I::I32Const(2));
	b.push(I::I32Add);
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(ACC));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(LEN));
	put(&mut b, 0x27); // opening '
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	// BYTE = source[I_] (unsigned).
	b.push(I::LocalGet(BUF));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGetU(bv));
	b.push(I::LocalSet(BYTE));
	b.push(I::LocalGet(BYTE));
	b.push(I::I32Const(0x5c));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	put(&mut b, 0x5c);
	put(&mut b, 0x5c);
	b.push(I::Else);
	b.push(I::LocalGet(BYTE));
	b.push(I::I32Const(0x27));
	b.push(I::I32Eq);
	b.push(I::If(empty));
	put(&mut b, 0x5c);
	put(&mut b, 0x27);
	b.push(I::Else);
	b.push(I::LocalGet(BYTE));
	b.push(I::I32Const(0x20));
	b.push(I::I32GeS);
	b.push(I::LocalGet(BYTE));
	b.push(I::I32Const(0x7e));
	b.push(I::I32LeS);
	b.push(I::I32And);
	b.push(I::If(empty));
	// printable: copy the byte verbatim.
	b.push(I::LocalGet(ACC));
	b.push(I::LocalGet(LEN));
	b.push(I::LocalGet(BYTE));
	b.push(I::ArraySet(bv));
	b.push(I::LocalGet(LEN));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(LEN));
	b.push(I::Else);
	put(&mut b, 0x5c); // '\'
	put(&mut b, 0x78); // 'x'
	put_hex(&mut b, 4);
	put_hex(&mut b, 0);
	b.push(I::End);
	b.push(I::End);
	b.push(I::End);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	put(&mut b, 0x27); // closing '
										// Trim ACC[0..LEN] into a tight $bytes (BUF), then wrap as $str.
	b.push(I::LocalGet(LEN));
	b.push(I::ArrayNewDefault(bv));
	b.push(I::LocalSet(BUF));
	b.push(I::LocalGet(BUF));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(ACC));
	b.push(I::I32Const(0));
	b.push(I::LocalGet(LEN));
	b.push(I::ArrayCopy {
		array_type_index_dst: bv,
		array_type_index_src: bv,
	});
	b.push(I::LocalGet(BUF));
	b.push(I::LocalSet(ACC));
	wrap(&mut b);
	b.push(I::End);

	// Element loop shared by TUPLE/LIST/RECORD: iterate ARR[0..N] appending
	// `__tostring(elem)` with `, ` separators. `pre`/`post` wrap the open/close.
	let elems_loop =
		|b: &mut Vec<Instruction>, arr: u32, open: (u32, u32), close: (u32, u32), record: bool| {
			// ACC = open
			lit_bytes(b, open);
			b.push(I::LocalSet(ACC));
			b.push(I::LocalGet(arr));
			b.push(I::ArrayLen);
			b.push(I::LocalSet(N));
			b.push(I::I32Const(0));
			b.push(I::LocalSet(I_));
			b.push(I::Block(empty));
			b.push(I::Loop(empty));
			b.push(I::LocalGet(I_));
			b.push(I::LocalGet(N));
			b.push(I::I32GeS);
			b.push(I::BrIf(1)); // -> end
											 // separator before all but the first
			b.push(I::LocalGet(I_));
			b.push(I::I32Const(0));
			b.push(I::I32GtS);
			b.push(I::If(empty));
			cat_lit(b, lits.comma_sp);
			b.push(I::End);
			if record {
				// "name: value": NAMES[i] is a raw $str; values in ARR.
				b.push(I::LocalGet(ACC));
				b.push(I::LocalGet(NAMES));
				b.push(I::LocalGet(I_));
				b.push(I::ArrayGet(types::T_VALARRAY));
				b.push(cast(types::T_STR));
				b.push(I::StructGet {
					struct_type_index: types::T_STR,
					field_index: 1,
				});
				b.push(I::Call(bc));
				b.push(I::LocalSet(ACC));
				cat_lit(b, lits.colon_sp);
			}
			cat_tostring_of(b, arr);
			b.push(I::LocalGet(I_));
			b.push(I::I32Const(1));
			b.push(I::I32Add);
			b.push(I::LocalSet(I_));
			b.push(I::Br(0));
			b.push(I::End); // loop
			b.push(I::End); // block
			cat_lit(b, close);
			wrap(b);
		};

	// TUPLE -> "(e, ...)".
	arm(&mut b, types::TAG_TUPLE);
	b.push(I::LocalGet(V));
	b.push(cast(types::T_TUPLE));
	b.push(I::StructGet {
		struct_type_index: types::T_TUPLE,
		field_index: 1,
	});
	b.push(I::LocalSet(ARR));
	elems_loop(&mut b, ARR, lits.lparen, lits.rparen, false);
	b.push(I::End);
	// LIST -> "[e, ...]".
	arm(&mut b, types::TAG_LIST);
	b.push(I::LocalGet(V));
	b.push(cast(types::T_LIST));
	b.push(I::StructGet {
		struct_type_index: types::T_LIST,
		field_index: 1,
	});
	b.push(I::LocalSet(ARR));
	elems_loop(&mut b, ARR, lits.lbrack, lits.rbrack, false);
	b.push(I::End);
	// RECORD -> "{k: v, ...}" (name-sorted; names raw, values via __tostring).
	arm(&mut b, types::TAG_RECORD);
	b.push(I::LocalGet(V));
	b.push(cast(types::T_RECORD));
	b.push(I::StructGet {
		struct_type_index: types::T_RECORD,
		field_index: 1,
	});
	b.push(I::LocalSet(NAMES));
	b.push(I::LocalGet(V));
	b.push(cast(types::T_RECORD));
	b.push(I::StructGet {
		struct_type_index: types::T_RECORD,
		field_index: 2,
	});
	b.push(I::LocalSet(ARR));
	elems_loop(&mut b, ARR, lits.lbrace, lits.rbrace, true);
	b.push(I::End);
	// VARIANT -> "enum.variant" then ` arg` per payload element.
	arm(&mut b, types::TAG_VARIANT);
	// ACC = bytes-of(name).
	b.push(I::LocalGet(V));
	b.push(cast(types::T_VARIANT));
	b.push(I::StructGet {
		struct_type_index: types::T_VARIANT,
		field_index: 2,
	});
	b.push(cast(types::T_STR));
	b.push(I::StructGet {
		struct_type_index: types::T_STR,
		field_index: 1,
	});
	b.push(I::LocalSet(ACC));
	b.push(I::LocalGet(V));
	b.push(cast(types::T_VARIANT));
	b.push(I::StructGet {
		struct_type_index: types::T_VARIANT,
		field_index: 3,
	});
	b.push(I::LocalSet(ARR));
	b.push(I::LocalGet(ARR));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	cat_lit(&mut b, lits.space);
	cat_tostring_of(&mut b, ARR);
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	wrap(&mut b);
	b.push(I::End);

	// REF -> "ref <inner>" (matches `vm::Value`'s Display).
	arm(&mut b, types::TAG_REF);
	// ACC = bytes-of "ref ".
	lit_bytes(&mut b, lits.ref_pfx);
	b.push(I::LocalSet(ACC));
	// ACC = bytesconcat(ACC, strbytes(tostring(cell))).
	b.push(I::LocalGet(ACC));
	b.push(I::LocalGet(V));
	b.push(cast(types::T_REF));
	b.push(I::StructGet {
		struct_type_index: types::T_REF,
		field_index: 1,
	});
	b.push(I::Call(self_idx)); // -> $str
	b.push(cast(types::T_STR));
	b.push(I::StructGet {
		struct_type_index: types::T_STR,
		field_index: 1,
	});
	b.push(I::Call(bc));
	b.push(I::LocalSet(ACC));
	wrap(&mut b);
	b.push(I::End);

	// DICT -> "{k: v, ...}" (insertion order; each entry a `$tuple`). Mirrors
	// `vm::Value`'s Dict Display.
	arm(&mut b, types::TAG_DICT);
	b.push(I::LocalGet(V));
	b.push(cast(types::T_DICT));
	b.push(I::StructGet {
		struct_type_index: types::T_DICT,
		field_index: 1,
	});
	b.push(I::LocalSet(ARR));
	// ACC = "{"  (set, not concat — ACC is not yet initialized here).
	lit_bytes(&mut b, lits.lbrace);
	b.push(I::LocalSet(ACC));
	b.push(I::LocalGet(ARR));
	b.push(I::ArrayLen);
	b.push(I::LocalSet(N));
	b.push(I::I32Const(0));
	b.push(I::LocalSet(I_));
	b.push(I::Block(empty));
	b.push(I::Loop(empty));
	b.push(I::LocalGet(I_));
	b.push(I::LocalGet(N));
	b.push(I::I32GeS);
	b.push(I::BrIf(1));
	// separator before all but the first
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(0));
	b.push(I::I32GtS);
	b.push(I::If(empty));
	cat_lit(&mut b, lits.comma_sp);
	b.push(I::End);
	// key: ACC ++ tostring(entry.elems[0])
	b.push(I::LocalGet(ACC));
	b.push(I::LocalGet(ARR));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(types::T_VALARRAY));
	b.push(cast(types::T_TUPLE));
	b.push(I::StructGet {
		struct_type_index: types::T_TUPLE,
		field_index: 1,
	});
	b.push(I::I32Const(0));
	b.push(I::ArrayGet(types::T_VALARRAY));
	b.push(I::Call(self_idx)); // -> $str
	b.push(cast(types::T_STR));
	b.push(I::StructGet {
		struct_type_index: types::T_STR,
		field_index: 1,
	});
	b.push(I::Call(bc));
	b.push(I::LocalSet(ACC));
	cat_lit(&mut b, lits.colon_sp);
	// value: ACC ++ tostring(entry.elems[1])
	b.push(I::LocalGet(ACC));
	b.push(I::LocalGet(ARR));
	b.push(I::LocalGet(I_));
	b.push(I::ArrayGet(types::T_VALARRAY));
	b.push(cast(types::T_TUPLE));
	b.push(I::StructGet {
		struct_type_index: types::T_TUPLE,
		field_index: 1,
	});
	b.push(I::I32Const(1));
	b.push(I::ArrayGet(types::T_VALARRAY));
	b.push(I::Call(self_idx)); // -> $str
	b.push(cast(types::T_STR));
	b.push(I::StructGet {
		struct_type_index: types::T_STR,
		field_index: 1,
	});
	b.push(I::Call(bc));
	b.push(I::LocalSet(ACC));
	b.push(I::LocalGet(I_));
	b.push(I::I32Const(1));
	b.push(I::I32Add);
	b.push(I::LocalSet(I_));
	b.push(I::Br(0));
	b.push(I::End); // loop
	b.push(I::End); // block
	cat_lit(&mut b, lits.rbrace);
	wrap(&mut b);
	b.push(I::End);

	// Unreachable: every value tag is handled above.
	b.push(I::Unreachable);
	let mut f = Function::new_with_locals_types(locals);
	for ins in &b {
		f.instruction(ins);
	}
	f.instruction(&I::End);
	f
}
