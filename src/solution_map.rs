use crate::constraint::*;
use crate::expr_type::*;
use std::collections::HashMap;
use Constraint::*;

pub struct SolutionMap {
  pub solutions: HashMap<usize, ExprType>,
}

impl SolutionMap {
  pub fn empty() -> Self {
    Self {
      solutions: HashMap::new(),
    }
  }

  pub fn with_entry(key: usize, value: ExprType) -> Self {
    let mut solutions = HashMap::with_capacity(1);
    solutions.insert(key, value);
    Self { solutions }
  }

  pub fn apply_to_constraints(&self, constraints: &[Constraint]) -> ConstraintSet {
    constraints
      .iter()
      .map(|con| match con {
        Eq(a, b) => Eq(
          a.replace_placeholders(&self.solutions),
          b.replace_placeholders(&self.solutions),
        ),
      })
      .collect()
  }

  pub fn compose(&self, other: SolutionMap) -> SolutionMap {
    let mut merged_solutions = HashMap::new();

    for (k, v) in &self.solutions {
      // add self.solutions with replacements from other
      merged_solutions.insert(*k, v.replace_placeholders(&other.solutions));
    }

    for (k, v) in &other.solutions {
      // add other.solutions
      merged_solutions.insert(*k, v.clone());
    }

    SolutionMap {
      solutions: merged_solutions,
    }
  }
}
