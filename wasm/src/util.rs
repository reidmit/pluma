// Small cross-cutting helpers shared by the assembler, the per-function emitter,
// and the string-pre-scan: the enum table alias, variant display-name rendering,
// and the IR-repr / binop -> wasm-type/instruction mappings.

use std::collections::HashMap;

use ir::Repr;
use wasm_encoder::{Instruction, ValType};

use crate::types;

/// The enum table the backend reads: fully-qualified enum name -> its ordered
/// variants (each `(name, payload-arity)`).
pub(crate) type EnumTable = HashMap<String, Vec<(String, usize)>>;

/// The display name of a variant — `bare-enum.variant`, matching `vm::Value`'s
/// `Display` (e.g. `tree.node`, `color.red`). Stored in each `$variant` so the
/// host formatter and `__tostring` can render it without a name table.
pub(crate) fn variant_display(enum_name: &str, tag: u32, enums: &EnumTable) -> String {
	let bare = enum_name.rsplit_once('.').map_or(enum_name, |(_, n)| n);
	let variant = enums
		.get(enum_name)
		.and_then(|vs| vs.get(tag as usize))
		.map_or("?", |(n, _)| n.as_str());
	format!("{bare}.{variant}")
}

/// Resolve a variant name to its within-enum tag across all enums (unique-name or
/// shared-tag assumption, as in `FnEmitter::variant_tag`).
pub(crate) fn variant_tag_in(enums: &EnumTable, name: &str) -> Option<u32> {
	let mut found = None;
	for vs in enums.values() {
		if let Some(i) = vs.iter().position(|(n, _)| n == name) {
			match found {
				None => found = Some(i as u32),
				Some(t) if t == i as u32 => {}
				Some(_) => return None,
			}
		}
	}
	found
}

pub(crate) fn repr_valtype(r: Repr) -> ValType {
	match r {
		Repr::Boxed => types::value_ref(),
		Repr::I64 => ValType::I64,
		Repr::F64 => ValType::F64,
		Repr::I32 => ValType::I32,
	}
}

pub(crate) fn binop_instr(op: ir::BinOp) -> Option<Instruction<'static>> {
	use ir::BinOp::*;
	Some(match op {
		AddInt => Instruction::I64Add,
		SubInt => Instruction::I64Sub,
		MulInt => Instruction::I64Mul,
		DivInt => Instruction::I64DivS,
		RemInt => Instruction::I64RemS,
		AddFloat => Instruction::F64Add,
		SubFloat => Instruction::F64Sub,
		MulFloat => Instruction::F64Mul,
		DivFloat => Instruction::F64Div,
		// f64 has no remainder opcode; `RemFloat` is lowered inline in `emit.rs`
		// as `a - trunc(a/b)*b` (it can't be a single instruction).
		RemFloat => return None,
		// Ordering comparisons, split by operand repr; result is i32 (bool).
		LtI64 => Instruction::I64LtS,
		LeI64 => Instruction::I64LeS,
		GtI64 => Instruction::I64GtS,
		GeI64 => Instruction::I64GeS,
		LtF64 => Instruction::F64Lt,
		LeF64 => Instruction::F64Le,
		GtF64 => Instruction::F64Gt,
		GeF64 => Instruction::F64Ge,
		// Strict logical and/or over i32 booleans (both operands evaluated).
		And => Instruction::I32And,
		Or => Instruction::I32Or,
		// Concrete numeric equality compares unboxed registers directly. Float
		// `==`/`!=` is IEEE (`nan != nan`), matching structural `==`/`!=` on floats.
		EqI64 => Instruction::I64Eq,
		NeI64 => Instruction::I64Ne,
		EqF64 => Instruction::F64Eq,
		NeF64 => Instruction::F64Ne,
		// Structural equality (any type) and string concat need runtime helpers.
		Eq | Ne | Concat => return None,
	})
}
