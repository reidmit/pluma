use super::*;

pub struct LiteralNode {
	pub pos: Position,
	pub kind: LiteralKind,
}

pub enum LiteralKind {
	FloatDecimal(f64),
	IntDecimal(i32),
	IntOctal(i32),
	IntHex(i32),
	IntBinary(i32),
	Str(String),
}

impl std::fmt::Debug for LiteralNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use LiteralKind::*;

		match &self.kind {
			FloatDecimal(val) => write!(f, "float:{}-{} {}", self.pos.0, self.pos.1, val),
			IntDecimal(val) => write!(f, "decimal:{}-{} {}", self.pos.0, self.pos.1, val),
			IntOctal(val) => write!(f, "octal:{}-{} {}", self.pos.0, self.pos.1, val),
			IntHex(val) => write!(f, "hex:{}-{} {}", self.pos.0, self.pos.1, val),
			IntBinary(val) => write!(f, "binary:{}-{} {}", self.pos.0, self.pos.1, val),
			Str(val) => write!(f, "string:{}-{} {:?}", self.pos.0, self.pos.1, val),
		}
	}
}
