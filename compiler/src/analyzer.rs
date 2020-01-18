#![allow(unused_variables)]

use crate::ast::{get_node_type, Node, NodeType};
use crate::errors::{AnalysisError, AnalysisError::*};
use crate::scope::Scope;

pub fn analyze_ast(node: &mut Option<Node>) -> Result<(), AnalysisError> {
  let mut state = AnalyzerState::new();

  match node {
    Some(node) => analyze(node, &mut state),
    _ => unreachable!(),
  }
}

fn analyze(node: &mut Node, state: &mut AnalyzerState) -> Result<(), AnalysisError> {
  match node {
    Node::Array {
      elements,
      inferred_type,
      ..
    } => {
      let mut first_element_type = None;

      for element in elements {
        analyze(element, state)?;

        let element_type = get_node_type(element);

        if let Some(first_type) = first_element_type.clone() {
          if first_type != element_type {
            return Err(TypeMismatchArrayElement(
              element.clone(),
              first_type,
              element_type,
            ));
          }
        } else {
          first_element_type = Some(element_type.clone());
        }
      }

      let type_params = match first_element_type {
        Some(element_type) => vec![element_type.clone()],
        None => vec![NodeType::Generic],
      };

      *inferred_type = NodeType::Identifier {
        name: "list".to_owned(),
        type_params,
      }
    }

    Node::Assignment {
      left,
      right,
      inferred_type,
      ..
    } => {
      analyze(right, state)?;

      let name = get_identifier_name(left);

      *inferred_type = get_node_type(right);

      state.scope.add(name, get_node_type(right));
    }

    Node::Block {
      params,
      body,
      inferred_type,
      ..
    } => {
      let mut param_types = vec![];
      let mut return_type = None;

      for param in params {
        analyze(param, state)?;
        param_types.push(get_node_type(param));
      }

      for expr in body {
        analyze(expr, state)?;
        return_type = Some(get_node_type(expr));
      }

      *inferred_type = NodeType::Function {
        param_types,
        return_type: Box::new(return_type.unwrap()),
      }
    }

    Node::Call {
      callee, arguments, ..
    } => {
      analyze(callee, state)?;

      for argument in arguments {
        analyze(argument, state)?;
      }
    }

    Node::Chain { start, end, .. } => {}

    Node::Dict { start, end, .. } => {}

    Node::DictEntry { start, end, .. } => {}

    Node::Grouping { start, end, .. } => {}

    Node::Identifier {
      name,
      inferred_type,
      ..
    } => match state.scope.get(name) {
      Some(node_type) => *inferred_type = node_type,
      None => return Err(UndefinedVariable(node.clone())),
    },

    Node::Import { start, end, .. } => {}

    Node::Match { start, end, .. } => {}

    Node::MatchCase { start, end, .. } => {}

    Node::MethodDefinition { start, end, .. } => {}

    Node::Module { body, .. } => {
      state.scope.enter();

      for body_node in body {
        analyze(body_node, state)?;
      }

      state.scope.exit();
    }

    Node::NumericLiteral { inferred_type, .. } => *inferred_type = NodeType::Int,

    Node::Reassignment { start, end, .. } => {}

    Node::StringInterpolation {
      parts,
      inferred_type,
      ..
    } => {
      for part in parts {
        analyze(part, state)?;

        match get_node_type(part) {
          NodeType::String => {}
          other => return Err(TypeMismatch(part.clone(), NodeType::String, other)),
        }
      }

      *inferred_type = NodeType::String
    }

    Node::StringLiteral { inferred_type, .. } => *inferred_type = NodeType::String,

    Node::Tuple { start, end, .. } => {}

    Node::UnaryOperation { start, end, .. } => {}
  }

  Ok(())
}

fn get_identifier_name(node: &Node) -> String {
  match node {
    Node::Identifier { name, .. } => name.to_string(),
    _ => unreachable!(),
  }
}

struct AnalyzerState {
  scope: Scope,
}

impl AnalyzerState {
  fn new() -> Self {
    AnalyzerState {
      scope: Scope::new(),
    }
  }
}