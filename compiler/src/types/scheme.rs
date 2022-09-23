use crate::types::*;
use std::collections::HashSet;

#[derive(Clone)]
pub enum Scheme {
	Var(usize),
	Forall(Vec<usize>, Type),
}

impl Scheme {
	pub fn free_vars(&self) -> HashSet<usize> {
		match self {
			Scheme::Var(_) => HashSet::new(),
			Scheme::Forall(vars, ty) => {
				let mut ty_free_vars = ty.free_vars();

				for var in vars {
					// remove quantified vars, since they aren't free
					ty_free_vars.remove(var);
				}

				ty_free_vars
			}
		}
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Scheme {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Scheme::Var(var) => write!(f, "scheme {}", Type::Var(*var)),
			Scheme::Forall(vars, ty) => write!(
				f,
				"forall{} . {}",
				vars.iter()
					.map(|v| format!(" {}", v))
					.collect::<Vec<String>>()
					.join(""),
				ty
			),
		}
	}
}
