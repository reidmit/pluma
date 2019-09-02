use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Node {
  Module {
    body: Vec<Node>,
    comments: HashMap<usize, Node>,
  },

  Block {
    line_start: usize,
    col_start: usize,
    line_end: usize,
    col_end: usize,
    params: Vec<Node>,
    body: Vec<Node>,
    inferred_type: NodeType,
  },

  Comment {
    line: usize,
    col_start: usize,
    col_end: usize,
    value: String,
  },

  Identifier {
    line: usize,
    col_start: usize,
    col_end: usize,
    name: String,
    inferred_type: NodeType,
  },

  IntLiteral {
    line: usize,
    col_start: usize,
    col_end: usize,
    value: String,
    inferred_type: NodeType,
  },

  StringLiteral {
    line_start: usize,
    line_end: usize,
    col_start: usize,
    col_end: usize,
    value: String,
    inferred_type: NodeType,
  },

  StringInterpolation {
    line_start: usize,
    line_end: usize,
    col_start: usize,
    col_end: usize,
    parts: Vec<Node>,
    inferred_type: NodeType,
  },

  Assignment {
    line_start: usize,
    line_end: usize,
    col_start: usize,
    col_end: usize,
    is_constant: bool,
    left: Box<Node>,
    right: Box<Node>,
    inferred_type: NodeType,
  },
}

#[derive(Debug, Clone)]
pub enum NodeType {
  Unknown,

  Func {
    param_types: Vec<NodeType>,
    return_type: Box<NodeType>,
  },
}

pub fn extract_location(node: &Node) -> (usize, usize, usize, usize) {
  match node {
    Node::Identifier {
      line,
      col_start,
      col_end,
      ..
    } => (*line, *line, *col_start, *col_end),

    Node::IntLiteral {
      line,
      col_start,
      col_end,
      ..
    } => (*line, *line, *col_start, *col_end),

    Node::StringLiteral {
      line_start,
      line_end,
      col_start,
      col_end,
      ..
    } => (*line_start, *line_end, *col_start, *col_end),

    Node::StringInterpolation {
      line_start,
      line_end,
      col_start,
      col_end,
      ..
    } => (*line_start, *line_end, *col_start, *col_end),

    Node::Assignment {
      line_start,
      line_end,
      col_start,
      col_end,
      ..
    } => (*line_start, *line_end, *col_start, *col_end),

    _ => (0, 0, 0, 0),
  }
}