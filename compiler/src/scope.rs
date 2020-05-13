use crate::analysis_error::{AnalysisError, AnalysisErrorKind};
use crate::diagnostics::Diagnostic;
use crate::types::ValueType;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug)]
pub struct Binding {
  pub typ: ValueType,
  pub ref_count: usize,
  pos: (usize, usize),
}

#[derive(Debug)]
struct ScopeLevel {
  pub let_bindings: HashMap<String, Binding>,
}

#[derive(Debug)]
pub struct Scope {
  pub type_bindings: HashMap<String, ValueType>,
  levels: Vec<ScopeLevel>,
}

impl Scope {
  pub fn new() -> Self {
    Scope {
      levels: Vec::new(),
      type_bindings: HashMap::new(),
    }
  }

  pub fn enter(&mut self) {
    self.levels.push(ScopeLevel {
      let_bindings: HashMap::new(),
    });
  }

  pub fn exit(&mut self) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    if let Some(exited_level) = self.levels.pop() {
      for (name, binding) in exited_level.let_bindings {
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

  pub fn add_let_binding(&mut self, name: String, typ: ValueType, pos: (usize, usize)) {
    let current_level = self.levels.last_mut().expect("no current scope");

    current_level.let_bindings.insert(
      name,
      Binding {
        typ,
        ref_count: 0,
        pos,
      },
    );
  }

  pub fn add_type_binding(&mut self, name: String, typ: ValueType) {
    self.type_bindings.insert(name, typ);
  }

  pub fn get_let_binding(&mut self, name: &String) -> Option<&ValueType> {
    for level in self.levels.iter_mut().rev() {
      if let Some(binding) = level.let_bindings.get_mut(name) {
        binding.ref_count += 1;

        return Some(&binding.typ);
      }
    }

    None
  }
}
