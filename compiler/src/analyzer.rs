use crate::ast::{Node, NodeType};
use crate::errors::{AnalysisError, AnalysisError::*};
use crate::scope::Scope;
use std::collections::HashMap;

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

        let element_type = element.get_type();

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

      *inferred_type = right.get_type();

      state.local_scope.add(name, right.get_type());
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
        param_types.push(param.get_type());
      }

      for expr in body {
        analyze(expr, state)?;
        return_type = Some(expr.get_type());
      }

      *inferred_type = NodeType::Function {
        param_types,
        return_type: Box::new(return_type.unwrap()),
      }
    }

    Node::Break { .. } => {}

    Node::Call {
      callee, arguments, ..
    } => {
      analyze(callee, state)?;

      for argument in arguments {
        analyze(argument, state)?;
      }
    }

    Node::Chain { .. } => {}

    Node::Dict { .. } => {}

    Node::DictEntry { .. } => {}

    Node::Grouping { .. } => {}

    Node::Identifier {
      name,
      inferred_type,
      ..
    } => match state.local_scope.get(name) {
      Some(node_type) => *inferred_type = node_type,
      None => return Err(UndefinedVariable(node.clone())),
    },

    Node::Import {
      alias, module_name, ..
    } => {
      if let Some(alias_name) = alias {
        state
          .module_aliases
          .insert(alias_name.to_string(), module_name.to_string());
      } else {
        // TODO: add all top-level defs in imported module to scope
      }
    }

    Node::Match { .. } => {}

    Node::MatchCase { .. } => {}

    Node::MethodDefinition { .. } => {}

    Node::Module { body, imports, .. } => {
      for import_node in imports {
        analyze(import_node, state)?;
      }

      state.local_scope.enter();

      for body_node in body {
        analyze(body_node, state)?;
      }

      state.local_scope.exit();
    }

    Node::NumericLiteral { inferred_type, .. } => *inferred_type = NodeType::Int,

    Node::PrivateMarker { .. } => {}

    Node::Reassignment { .. } => {}

    Node::Return { .. } => {}

    Node::StringInterpolation {
      parts,
      inferred_type,
      ..
    } => {
      for part in parts {
        analyze(part, state)?;

        match part.get_type() {
          NodeType::String => {}
          other => return Err(TypeMismatch(part.clone(), NodeType::String, other)),
        }
      }

      *inferred_type = NodeType::String
    }

    Node::StringLiteral { inferred_type, .. } => *inferred_type = NodeType::String,

    Node::TraitDefinition { .. } => {}

    Node::Tuple { .. } => {}

    Node::TypeDefinition { .. } => {}

    _ => panic!("yikes"),
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
  local_scope: Scope,
  module_aliases: HashMap<String, String>,
}

impl AnalyzerState {
  fn new() -> Self {
    AnalyzerState {
      local_scope: Scope::new(),
      module_aliases: HashMap::new(),
    }
  }
}
