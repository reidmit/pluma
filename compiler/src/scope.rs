use crate::ast::NodeType;
use std::collections::HashMap;

#[derive(Debug)]
pub struct Scope {
  variables: Vec<HashMap<String, NodeType>>,
}

impl Scope {
  pub fn new() -> Self {
    Scope {
      variables: Vec::new(),
    }
  }

  pub fn enter(&mut self) {
    self.variables.push(HashMap::new());
  }

  pub fn exit(&mut self) {
    self.variables.pop();
  }

  pub fn add(&mut self, name: String, node_type: NodeType) {
    if let Some(map) = self.variables.last_mut() {
      map.insert(name, node_type);
    }
  }

  pub fn get(&self, name: &String) -> Option<NodeType> {
    for level in self.variables.iter().rev() {
      if let Some(value) = level.get(name) {
        return Some(value.to_owned());
      }
    }

    None
  }
}