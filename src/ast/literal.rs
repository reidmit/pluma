use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct LiteralNode {
	pub span: Span,
	pub kind: LiteralKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum LiteralKind {
	FloatDecimal(f64),
	IntDecimal(usize),
	IntOctal(usize),
	IntHex(usize),
	IntBinary(usize),
	Str(String),
}
