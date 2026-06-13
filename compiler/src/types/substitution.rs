use crate::types::*;
use std::collections::HashMap;

// A solution for a row variable: "the row var stands for these extra fields,
// with this tail." A `None` tail means "and nothing else" (closed extension);
// `Some(rid)` means the row variable's tail is itself another row variable
// (typically introduced by unifying two open records).
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct RowSolution {
	pub fields: Vec<(String, Type)>,
	pub tail: Option<usize>,
}

// The tuple analogue of `RowSolution`: a row variable for an open `PartialTuple`
// stands for these extra `(index, type)` pairs, with this tail. Kept in a
// separate map from `RowSolution` since the keys are tuple indices, not field
// names; row-variable ids are globally unique, so a given id only ever appears
// in one of the two maps.
#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TupleRowSolution {
	pub fields: Vec<(usize, Type)>,
	pub tail: Option<usize>,
}

#[derive(Clone)]
pub struct Substitution {
	pub solutions: HashMap<usize, Type>,
	// Solutions for record row variables — distinct namespace from `solutions`.
	pub row_solutions: HashMap<usize, RowSolution>,
	// Solutions for tuple row variables — see `TupleRowSolution`.
	pub tuple_row_solutions: HashMap<usize, TupleRowSolution>,
}

impl Substitution {
	pub fn empty() -> Self {
		Self {
			solutions: HashMap::new(),
			row_solutions: HashMap::new(),
			tuple_row_solutions: HashMap::new(),
		}
	}

	pub fn with_entry(key: usize, value: Type) -> Self {
		let mut solutions = HashMap::with_capacity(1);
		solutions.insert(key, value);
		Self {
			solutions,
			row_solutions: HashMap::new(),
			tuple_row_solutions: HashMap::new(),
		}
	}

	pub fn with_row_entry(key: usize, value: RowSolution) -> Self {
		let mut row_solutions = HashMap::with_capacity(1);
		row_solutions.insert(key, value);
		Self {
			solutions: HashMap::new(),
			row_solutions,
			tuple_row_solutions: HashMap::new(),
		}
	}

	/// Recover the substitution mapping `generic`'s quantified type/row variables to
	/// the concrete subtypes in `concrete`, by a congruent-shape diff walk. The two
	/// types are assumed congruent — they came from the same scheme, one
	/// freshened-and-solved — so this is NOT unification: a structural mismatch on a
	/// branch simply records nothing for it. Wherever `generic` has a `Var(v)`, bind
	/// `v` to the concrete subtype; wherever it has an open record/tuple row tail,
	/// bind that row variable to the concrete's extra fields. First binding wins.
	/// Used by record-shape monomorphization to turn a call site's instantiation into
	/// a substitution it can re-lower the callee body under.
	pub fn congruent_diff(generic: &Type, concrete: &Type) -> Substitution {
		let mut s = Substitution::empty();
		s.diff_into(generic, concrete);
		s
	}

	fn diff_into(&mut self, generic: &Type, concrete: &Type) {
		match (generic, concrete) {
			(Type::Var(v), c) => {
				self.solutions.entry(*v).or_insert_with(|| c.clone());
			}
			(Type::Fun(gp, gr), Type::Fun(cp, cr)) => {
				for (g, c) in gp.iter().zip(cp) {
					self.diff_into(g, c);
				}
				self.diff_into(gr, cr);
			}
			(Type::List(g), Type::List(c)) => self.diff_into(g, c),
			(Type::Dict(gk, gv), Type::Dict(ck, cv)) => {
				self.diff_into(gk, ck);
				self.diff_into(gv, cv);
			}
			(Type::Ref(g), Type::Ref(c)) => self.diff_into(g, c),
			(Type::Tuple(gs), Type::Tuple(cs)) => {
				for (g, c) in gs.iter().zip(cs) {
					self.diff_into(g, c);
				}
			}
			(Type::Enum(_, gs), Type::Enum(_, cs)) => {
				for (g, c) in gs.iter().zip(cs) {
					self.diff_into(g, c);
				}
			}
			(Type::Record(gf, gtail), Type::Record(cf, ctail)) => {
				for (gname, gty) in gf {
					if let Some((_, cty)) = cf.iter().find(|(n, _)| n == gname) {
						self.diff_into(gty, cty);
					}
				}
				// The generic's open row tail stands for whatever fields the concrete
				// carries beyond the ones named in the generic.
				if let Some(rid) = gtail {
					let fields: Vec<(String, Type)> = cf
						.iter()
						.filter(|(n, _)| !gf.iter().any(|(gn, _)| gn == n))
						.cloned()
						.collect();
					self.row_solutions.entry(*rid).or_insert(RowSolution {
						fields,
						tail: *ctail,
					});
				}
			}
			(Type::PartialTuple(gf, gtail), Type::Tuple(cs)) => {
				for (idx, gty) in gf {
					if let Some(cty) = cs.get(*idx) {
						self.diff_into(gty, cty);
					}
				}
				if let Some(rid) = gtail {
					let fields: Vec<(usize, Type)> = cs
						.iter()
						.enumerate()
						.filter(|(i, _)| !gf.iter().any(|(gi, _)| gi == i))
						.map(|(i, t)| (i, t.clone()))
						.collect();
					self
						.tuple_row_solutions
						.entry(*rid)
						.or_insert(TupleRowSolution { fields, tail: None });
				}
			}
			(Type::PartialTuple(gf, _), Type::PartialTuple(cf, _)) => {
				for (idx, gty) in gf {
					if let Some((_, cty)) = cf.iter().find(|(i, _)| i == idx) {
						self.diff_into(gty, cty);
					}
				}
			}
			_ => {}
		}
	}

	pub fn apply_to_type(&self, ty: &Type) -> Type {
		match ty {
			Type::Unknown
			| Type::Nothing
			| Type::Bool
			| Type::Int
			| Type::Float
			| Type::String
			| Type::Bytes
			| Type::Instant
			| Type::Duration => ty.clone(),

			Type::Var(var) => {
				if self.solutions.contains_key(var) {
					self.solutions.get(var).unwrap().clone()
				} else {
					ty.clone()
				}
			}

			Type::Enum(name, args) => Type::Enum(
				name.clone(),
				args.iter().map(|t| self.apply_to_type(t)).collect(),
			),

			Type::Fun(param_types, return_type) => Type::Fun(
				param_types.iter().map(|t| self.apply_to_type(t)).collect(),
				self.apply_to_type(return_type).into(),
			),

			Type::PartialTuple(field_types, tail) => {
				// Mirror the `Record` case: substitute through each known index's
				// type, then chase the tail through `tuple_row_solutions`,
				// merging in any indices each step pins down.
				let mut new_fields: Vec<(usize, Type)> = field_types
					.iter()
					.map(|(index, t)| (*index, self.apply_to_type(t)))
					.collect();
				let mut current_tail = *tail;
				while let Some(rid) = current_tail {
					match self.tuple_row_solutions.get(&rid) {
						Some(sol) => {
							for (index, t) in &sol.fields {
								new_fields.push((*index, self.apply_to_type(t)));
							}
							current_tail = sol.tail;
						}
						None => break,
					}
				}
				Type::partial_tuple(new_fields, current_tail)
			}

			Type::Tuple(element_types) => Type::Tuple(
				element_types
					.iter()
					.map(|t| self.apply_to_type(t))
					.collect(),
			),

			Type::Record(field_types, tail) => {
				// Substitute through each field's type.
				let mut new_fields: Vec<(String, Type)> = field_types
					.iter()
					.map(|(name, t)| (name.clone(), self.apply_to_type(t)))
					.collect();
				// Resolve the tail. Walk row solutions transitively — each
				// step may merge in more fields and replace the tail with
				// another row var (or finally close it with `None`).
				let mut current_tail = *tail;
				while let Some(rid) = current_tail {
					match self.row_solutions.get(&rid) {
						Some(sol) => {
							for (n, t) in &sol.fields {
								new_fields.push((n.clone(), self.apply_to_type(t)));
							}
							current_tail = sol.tail;
						}
						None => break,
					}
				}
				Type::Record(new_fields, current_tail)
			}

			Type::List(element_type) => Type::List(self.apply_to_type(element_type).into()),

			Type::Dict(key_type, value_type) => Type::Dict(
				self.apply_to_type(key_type).into(),
				self.apply_to_type(value_type).into(),
			),

			Type::Ref(inner_type) => Type::Ref(self.apply_to_type(inner_type).into()),
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
				Inst(var, ty, sink, range) => Inst(*var, self.apply_to_type(ty), sink.clone(), *range),
				Class(c) => Class(ClassConstraint {
					name: c.name.clone(),
					ty: self.apply_to_type(&c.ty),
					reason: c.reason.clone(),
					// The cell is shared with the AST — clone keeps the Rc.
					dispatch_cell: c.dispatch_cell.clone(),
				}),
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

		// Row solutions compose the same way: apply the new substitution to
		// existing row entries' field types and tail (chase through), then
		// add the new entries.
		let mut merged_rows: HashMap<usize, RowSolution> = HashMap::new();
		for (k, v) in &self.row_solutions {
			let new_fields: Vec<(String, Type)> = v
				.fields
				.iter()
				.map(|(n, t)| (n.clone(), other.apply_to_type(t)))
				.collect();
			// Chase the tail through `other`'s row solutions one step, in
			// case `other` resolves this tail further. The apply_to_type
			// path above already handles transitive chasing for full types;
			// here we just need to peek one step for the bare tail id.
			let new_tail = match v.tail {
				Some(t) if other.row_solutions.contains_key(&t) => {
					// Inline `other`'s resolution by composing fields.
					let other_sol = other.row_solutions.get(&t).unwrap();
					let mut combined = new_fields.clone();
					for (n, t) in &other_sol.fields {
						combined.push((n.clone(), other.apply_to_type(t)));
					}
					merged_rows.insert(
						*k,
						RowSolution {
							fields: combined,
							tail: other_sol.tail,
						},
					);
					continue;
				}
				other => other,
			};
			merged_rows.insert(
				*k,
				RowSolution {
					fields: new_fields,
					tail: new_tail,
				},
			);
		}
		for (k, v) in &other.row_solutions {
			merged_rows.entry(*k).or_insert_with(|| v.clone());
		}

		// Tuple row solutions compose identically to record row solutions above,
		// just over `(usize, Type)` field lists.
		let mut merged_tuple_rows: HashMap<usize, TupleRowSolution> = HashMap::new();
		for (k, v) in &self.tuple_row_solutions {
			let new_fields: Vec<(usize, Type)> = v
				.fields
				.iter()
				.map(|(i, t)| (*i, other.apply_to_type(t)))
				.collect();
			let new_tail = match v.tail {
				Some(t) if other.tuple_row_solutions.contains_key(&t) => {
					let other_sol = other.tuple_row_solutions.get(&t).unwrap();
					let mut combined = new_fields.clone();
					for (i, t) in &other_sol.fields {
						combined.push((*i, other.apply_to_type(t)));
					}
					merged_tuple_rows.insert(
						*k,
						TupleRowSolution {
							fields: combined,
							tail: other_sol.tail,
						},
					);
					continue;
				}
				other => other,
			};
			merged_tuple_rows.insert(
				*k,
				TupleRowSolution {
					fields: new_fields,
					tail: new_tail,
				},
			);
		}
		for (k, v) in &other.tuple_row_solutions {
			merged_tuple_rows.entry(*k).or_insert_with(|| v.clone());
		}

		Substitution {
			solutions: merged_solutions,
			row_solutions: merged_rows,
			tuple_row_solutions: merged_tuple_rows,
		}
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Substitution {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"types={:?} rows={:?} tuple_rows={:?}",
			self.solutions, self.row_solutions, self.tuple_row_solutions
		)
	}
}
