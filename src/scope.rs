use crate::diagnostic::*;
use crate::errors::*;
use crate::value_type::*;
use std::collections::HashMap;

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
	EnumVariant,
	StructConstructor,
	Field,
}

struct ScopeLevel {
	pub bindings: HashMap<String, Binding>,
}

pub struct Scope {
	levels: Vec<ScopeLevel>,
}

impl Scope {
	pub fn new() -> Self {
		Scope { levels: Vec::new() }
	}

	pub fn enter(&mut self) {
		self.levels.push(ScopeLevel {
			bindings: HashMap::new(),
		});
	}

	pub fn exit(&mut self) -> Result<(), Vec<Diagnostic>> {
		let mut diagnostics = Vec::new();

		if let Some(exited_level) = self.levels.pop() {
			for (name, binding) in exited_level.bindings {
				if binding.ref_count == 0 {
					diagnostics.push(
						Diagnostic::warning(AnalysisError {
							pos: binding.pos,
							kind: AnalysisErrorKind::UnusedVariable(name),
						})
						.with_pos(binding.pos),
					)
				}
			}
		}

		if diagnostics.len() > 0 {
			return Err(diagnostics);
		}

		Ok(())
	}

	pub fn add_binding(
		&mut self,
		kind: BindingKind,
		name: String,
		typ: ValueType,
		pos: (usize, usize),
	) {
		let current_level = self.levels.last_mut().expect("no current scope");

		current_level.bindings.insert(
			name,
			Binding {
				typ,
				ref_count: 0,
				pos,
				kind,
			},
		);
	}

	pub fn get_binding(&mut self, name: &String) -> Option<&Binding> {
		for level in self.levels.iter_mut().rev() {
			if let Some(binding) = level.bindings.get_mut(name) {
				binding.ref_count += 1;

				return Some(binding);
			}
		}

		None
	}
}
