use crate::ast::NodeType;
use std::collections::HashMap;

#[derive(Debug)]
pub struct Scope {
  levels: Vec<HashMap<String, NodeType>>,
}

impl Scope {
  pub fn new() -> Self {
    Scope { levels: Vec::new() }
  }

  pub fn enter(&mut self) {
    self.levels.push(HashMap::new());
  }

  pub fn exit(&mut self) {
    self.levels.pop();
  }

  pub fn add(&mut self, name: String, node_type: NodeType) {
    if let Some(map) = self.levels.last_mut() {
      map.insert(name, node_type);
    }
  }

  pub fn get(&self, name: &String) -> Option<NodeType> {
    for level in self.levels.iter().rev() {
      if let Some(value) = level.get(name) {
        return Some(value.to_owned());
      }
    }

    None
  }
}
