#![allow(unused_variables)]

use std::collections::HashMap;
use crate::ast::{Node, NodeType, get_node_type};
use crate::errors::{AnalysisError, AnalysisError::*};

pub fn analyze_ast(node: &mut Option<Node>) -> Result<(), AnalysisError> {
  let mut state = AnalyzerState::new();

  match node {
    Some(node) => analyze(node, &mut state),
    _ => unreachable!()
  }
}

fn analyze(node: &mut Node, state: &mut AnalyzerState) -> Result<(), AnalysisError> {
  match node {
    Node::Array { elements, .. } => {
      for element in elements {
        analyze(element, state)?;
      }
    },

    Node::Assignment { left, right, inferred_type, .. } => {
      analyze(right, state)?;

      let name = get_identifier_name(left);

      *inferred_type = get_node_type(right);

      state.scope.add(name, get_node_type(right));
    },

    Node::Block { params, body, .. } => {
      for param in params {
        analyze(param, state)?;
      }

      for expr in body {
        analyze(expr, state)?;
      }
    },

    Node::Call { callee, arguments, .. } => {
      analyze(callee, state)?;

      for argument in arguments {
        analyze(argument, state)?;
      }
    },

    Node::Chain { start, end, .. } => {

    },

    Node::Dict { start, end, .. } => {

    },

    Node::DictEntry { start, end, .. } => {

    },

    Node::Grouping { start, end, .. } => {

    },

    Node::Identifier { name, inferred_type, .. } => {
      match state.scope.get(name) {
        Some(node_type) => {
          *inferred_type = node_type
        },
        None => panic!("Not defined: {}", name)
      }
    },

    Node::Import { start, end, .. } => {

    },

    Node::Match { start, end, .. } => {

    },

    Node::MatchCase { start, end, .. } => {

    },

    Node::MethodDefinition { start, end, .. } => {

    },

    Node::Module { body, .. } => {
      state.scope.enter();

      for body_node in body {
        analyze(body_node, state)?;
      }

      state.scope.exit();
    },

    Node::NumericLiteral { inferred_type, .. } => {
      *inferred_type = NodeType::Int
    },

    Node::Reassignment { start, end, .. } => {

    },

    Node::StringInterpolation { parts, inferred_type, .. } => {
      for part in parts {
        analyze(part, state)?;

        match get_node_type(part) {
          NodeType::String => {},
          other => return Err(TypeMismatch(
            part.clone(),
            NodeType::String,
            other
          ))
        }
      }

      *inferred_type = NodeType::String
    },

    Node::StringLiteral { inferred_type, .. } => {
      *inferred_type = NodeType::String
    },

    Node::Tuple { start, end, .. } => {

    },

    Node::UnaryOperation { start, end, .. } => {

    },
  }

  Ok(())
}

fn get_identifier_name(node: &Node) -> String {
  match node {
    Node::Identifier { name, .. } => name.to_string(),
    _ => unreachable!()
  }
}

struct AnalyzerState {
  scope: Scope,
}

impl AnalyzerState {
  fn new() -> Self {
    AnalyzerState {
      scope: Scope::new()
    }
  }
}

#[derive(Debug)]
struct Scope {
  variables: Vec<HashMap<String, NodeType>>,
}

impl Scope {
  fn new() -> Self {
    Scope {
      variables: Vec::new()
    }
  }

  fn enter(&mut self) {
    self.variables.push(HashMap::new());
  }

  fn exit(&mut self) {
    self.variables.pop();
  }

  fn add(&mut self, name: String, node_type: NodeType) {
    if let Some(map) = self.variables.last_mut() {
      map.insert(name, node_type);
    }
  }

  fn get(&self, name: &String) -> Option<NodeType> {
    for level in self.variables.iter().rev() {
      if level.contains_key(name) {
        return Some(level.get(name).unwrap().to_owned());
      }
    }

    None
  }
}