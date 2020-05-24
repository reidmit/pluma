use crate::analysis_error::{AnalysisError, AnalysisErrorKind};
use crate::diagnostics::Diagnostic;
use pluma_ast::value_type::ValueType;
use std::collections::HashMap;

#[derive(Debug)]
pub struct Binding {
  pub typ: ValueType,
  pub ref_count: usize,
  pub pos: (usize, usize),
  pub kind: BindingKind,
}

#[derive(Debug)]
pub struct TypeBinding {
  pub typ: ValueType,
  pub ref_count: usize,
  pub pos: (usize, usize),
  pub kind: TypeBindingKind,
}

#[derive(Debug, PartialEq)]
pub enum BindingKind {
  Let,
  Def,
  Param,
  EnumVariant,
  StructConstructor,
}

#[derive(Debug, PartialEq)]
pub enum TypeBindingKind {
  Enum,
  Struct,
  Alias,
  Trait,
  IntrinsicType,
}

#[derive(Debug)]
struct ScopeLevel {
  pub bindings: HashMap<String, Binding>,
  pub type_bindings: HashMap<String, TypeBinding>,
}

#[derive(Debug)]
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
      type_bindings: HashMap::new(),
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

  pub fn add_type_binding(
    &mut self,
    kind: TypeBindingKind,
    name: String,
    typ: ValueType,
    pos: (usize, usize),
  ) {
    let current_level = self.levels.last_mut().expect("no current scope");

    current_level.type_bindings.insert(
      name,
      TypeBinding {
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

  pub fn get_type_binding(&mut self, name: &String) -> Option<&TypeBinding> {
    for level in self.levels.iter_mut().rev() {
      if let Some(binding) = level.type_bindings.get_mut(name) {
        binding.ref_count += 1;

        return Some(binding);
      }
    }

    None
  }
}
