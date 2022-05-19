use crate::expr_type::*;

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct ValueBinding {
	pub typ: ExprType,
	pub ref_count: usize,
	pub loc: (usize, usize),
}

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct TypeBinding {
	pub typ: ExprType,
	pub loc: (usize, usize),
}
