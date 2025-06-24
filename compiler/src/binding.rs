use crate::{location::Range, types::*};

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct ValueBinding {
	pub ty_scheme: Scheme,
	pub ref_count: usize,
	pub range: Range,
}

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct TypeBinding {
	pub ty: Type,
	pub ref_count: usize,
	pub _range: Range,
}
