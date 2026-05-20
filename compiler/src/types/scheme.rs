use crate::types::*;
use std::collections::HashSet;

#[derive(Clone)]
pub enum Scheme {
	Var(usize),
	// `Forall(type_vars, row_vars, constraints, ty)`. `constraints` are
	// class constraints (e.g. `Numeric a`) that quantify over `type_vars` —
	// every instantiation of this scheme adds fresh copies. `row_vars` are
	// row-variable ids quantified by this scheme (for row-polymorphic record
	// types). Both lists are empty for non-overloaded, non-row-polymorphic
	// schemes.
	Forall(Vec<usize>, Vec<usize>, Vec<ClassConstraint>, Type),
}

impl Scheme {
	pub fn free_vars(&self) -> HashSet<usize> {
		match self {
			Scheme::Var(_) => HashSet::new(),
			Scheme::Forall(vars, _row_vars, _constraints, ty) => {
				let mut ty_free_vars = ty.free_vars();

				for var in vars {
					// remove quantified vars, since they aren't free
					ty_free_vars.remove(var);
				}

				ty_free_vars
			}
		}
	}

	pub fn free_row_vars(&self) -> HashSet<usize> {
		match self {
			Scheme::Var(_) => HashSet::new(),
			Scheme::Forall(_vars, row_vars, _constraints, ty) => {
				let mut frv = ty.free_row_vars();
				for rv in row_vars {
					frv.remove(rv);
				}
				frv
			}
		}
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Scheme {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Scheme::Var(var) => write!(f, "scheme {}", Type::Var(*var)),
			Scheme::Forall(vars, row_vars, constraints, ty) => {
				write!(
					f,
					"forall{}",
					vars
						.iter()
						.map(|v| format!(" {}", v))
						.collect::<Vec<String>>()
						.join(""),
				)?;
				if !row_vars.is_empty() {
					write!(
						f,
						" rows{}",
						row_vars
							.iter()
							.map(|v| format!(" ρ{}", v))
							.collect::<Vec<String>>()
							.join("")
					)?;
				}
				if !constraints.is_empty() {
					write!(
						f,
						" [{}]",
						constraints
							.iter()
							.map(|c| format!("{} {}", c.name, c.ty))
							.collect::<Vec<String>>()
							.join(", ")
					)?;
				}
				write!(f, " . {}", ty)
			}
		}
	}
}
