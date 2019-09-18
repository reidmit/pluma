use crate::ast::{Node};

pub fn analyze(node: &mut Option<Node>) {
  match node {
    Some(node) => analyze_impl(node),
    _ => {}
  }
}

fn analyze_impl(node: &mut Node) {
  match node {
    Node::Module { body, .. } => {
      for body_node in body {
        analyze_impl(body_node);
      }
    },

    // Node::StringLiteral { inferred_type, .. } => {
    //   *inferred_type = NodeType::Named("String")
    // },

    _ => {}
  }
}