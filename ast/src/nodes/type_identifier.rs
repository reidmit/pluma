use super::*;
use crate::common::*;
use crate::value_type::*;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TypeIdentifierNode {
	pub pos: Position,
	pub name: String,
	pub generics: Vec<TypeExprNode>,
	pub constraints: Option<Vec<TypeConstraint>>,
}

impl TypeIdentifierNode {
	pub fn add_constraint(&mut self, constraint: TypeConstraint) {
		match &mut self.constraints {
			None => self.constraints = Some(vec![constraint]),
			Some(constraints) => constraints.push(constraint),
		}
	}
}
