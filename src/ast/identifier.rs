use super::*;

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct IdentifierNode {
	pub loc: Location,
	pub name: String,
}
