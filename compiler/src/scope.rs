use crate::types::ValueType;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug)]
pub struct Binding {
  pub node_id: Uuid,
}

#[derive(Debug)]
struct ScopeLevel {}

#[derive(Debug)]
pub struct Scope {
  pub type_bindings: HashMap<String, ValueType>,
  pub let_bindings: HashMap<String, ValueType>,
}

impl Scope {
  pub fn new() -> Self {
    Scope {
      type_bindings: HashMap::new(),
      let_bindings: HashMap::new(),
    }
  }

  pub fn add_let_binding(&mut self, name: String, typ: ValueType) {
    self.let_bindings.insert(name, typ);
  }

  pub fn add_type_binding(&mut self, name: String, typ: ValueType) {
    self.type_bindings.insert(name, typ);
  }

  pub fn get_let_binding(&self, name: &String) -> Option<&ValueType> {
    self.let_bindings.get(name)
  }
}
