use crate::types::*;
use std::collections::HashMap;

pub struct Substitution {
	pub solutions: HashMap<usize, Type>,
}

impl Substitution {
	pub fn empty() -> Self {
		Self {
			solutions: HashMap::new(),
		}
	}

	pub fn with_entry(key: usize, value: Type) -> Self {
		let mut solutions = HashMap::with_capacity(1);
		solutions.insert(key, value);
		Self { solutions }
	}

	pub fn apply_to_type(&self, ty: &Type) -> Type {
		match ty {
			Type::Unknown
			| Type::Nothing
			| Type::Bool
			| Type::Int
			| Type::Float
			| Type::String
			| Type::Regex => ty.clone(),

			Type::Var(var) => {
				if self.solutions.contains_key(var) {
					self.solutions.get(var).unwrap().clone()
				} else {
					ty.clone()
				}
			}

			Type::Enum(name) => Type::Enum(name.clone()),

			Type::Fun(param_types, return_type) => Type::Fun(
				param_types.iter().map(|t| self.apply_to_type(t)).collect(),
				self.apply_to_type(return_type).into(),
			),

			Type::PartialTuple(index, element_type) => {
				Type::PartialTuple(*index, self.apply_to_type(element_type).into())
			}

			Type::PartialRecord(field_name, field_type) => {
				Type::PartialRecord(field_name.clone(), self.apply_to_type(field_type).into())
			}

			Type::Tuple(element_types) => Type::Tuple(
				element_types
					.iter()
					.map(|t| self.apply_to_type(t))
					.collect(),
			),

			Type::Record(field_types) => Type::Record(
				field_types
					.iter()
					.map(|(name, t)| (name.clone(), self.apply_to_type(t)))
					.collect(),
			),

			Type::List(element_type) => Type::List(self.apply_to_type(element_type).into()),
		}
	}

	pub fn apply_to_constraints(&self, constraints: &[Constraint]) -> Vec<Constraint> {
		use Constraint::*;

		constraints
			.iter()
			.map(|con| match con {
				Eq(a, b, data) => Eq(self.apply_to_type(a), self.apply_to_type(b), data.clone()),
				// TODO: should we have a context arg here as well?
				// see https://github.com/igstan/linguae/blob/7e806dd121c21ed35187377fe3bd92d29d6150e6/lingua-002-hm-inference-sml/src/constraint.sml#L21
				Gen(scheme, ty) => Gen(scheme.clone(), self.apply_to_type(ty)),
				Inst(var, ty) => Inst(*var, self.apply_to_type(ty)),
			})
			.collect()
	}

	pub fn compose(&self, other: Substitution) -> Substitution {
		let mut merged_solutions = HashMap::new();

		for (k, v) in &self.solutions {
			// add self.solutions with replacements from other
			merged_solutions.insert(*k, other.apply_to_type(v));
		}

		for (k, v) in &other.solutions {
			// add other.solutions
			merged_solutions.insert(*k, v.clone());
		}

		Substitution {
			solutions: merged_solutions,
		}
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Substitution {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{:?}", self.solutions)
	}
}
