use crate::location::*;

#[derive(Clone)]
pub struct LiteralNode {
	pub kind: LiteralKind,
	pub range: Range,
}

#[derive(Clone)]
pub enum LiteralKind {
	Bool(bool),
	FloatDecimal(f64),
	/// A time duration, stored as a whole number of nanoseconds. Built from
	/// unit-suffixed literals like `5s` or `2m20s`.
	Duration(i64),
	IntDecimal(usize),
	IntOctal(usize),
	IntHex(usize),
	IntBinary(usize),
	/// A string literal. The bool records whether the source used the
	/// triple-quoted (`"""..."""`) form, so the formatter can preserve it.
	/// It carries no semantic meaning — analysis and lowering ignore it.
	String(String, bool),
	Bytes(Vec<u8>),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for LiteralNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "literal({:#?}) {:#?}", self.range, self.kind)
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for LiteralKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use LiteralKind::*;

		match &self {
			Bool(v) => write!(f, "bool {}", v),
			FloatDecimal(v) => write!(f, "float {}", v),
			Duration(v) => write!(f, "duration {}ns", v),
			IntDecimal(v) => write!(f, "int {}", v),
			IntHex(v) => write!(f, "hex int {}", v),
			IntOctal(v) => write!(f, "octal int {}", v),
			IntBinary(v) => write!(f, "binary int {}", v),
			String(v, _) => write!(f, "string \"{}\"", v),
			Bytes(b) => {
				write!(f, "bytes '")?;
				for &byte in b.iter() {
					match byte {
						b'\\' => write!(f, "\\\\")?,
						b'\'' => write!(f, "\\'")?,
						0x20..=0x7e => write!(f, "{}", byte as char)?,
						_ => write!(f, "\\x{:02x}", byte)?,
					}
				}
				write!(f, "'")
			}
		}
	}
}
