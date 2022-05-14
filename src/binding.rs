use crate::value_type::*;

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct Binding {
	pub typ: ValueType,
	pub ref_count: usize,
	pub pos: (usize, usize),
	pub kind: BindingKind,
}

#[derive(PartialEq, Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum BindingKind {
	Def,
	Let,
	Param,
}
