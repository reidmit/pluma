use super::*;

pub struct LiteralNode {
	pub span: Span,
	pub kind: LiteralKind,
}

pub enum LiteralKind {
	Bool(bool),
	FloatDecimal(f64),
	IntDecimal(usize),
	IntOctal(usize),
	IntHex(usize),
	IntBinary(usize),
	String(String),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for LiteralNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"literal({}-{}) {:#?}",
			self.span.0, self.span.1, self.kind
		)
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for LiteralKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use LiteralKind::*;

		match &self {
			Bool(v) => write!(f, "bool {}", v),
			FloatDecimal(v) => write!(f, "float {}", v),
			IntDecimal(v) => write!(f, "int {}", v),
			IntHex(v) => write!(f, "hex int {}", v),
			IntOctal(v) => write!(f, "octal int {}", v),
			IntBinary(v) => write!(f, "binary int {}", v),
			String(v) => write!(f, "string \"{}\"", v),
		}
	}
}
