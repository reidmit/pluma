#[derive(Debug)]
pub enum Node {
  Module {
    body: Vec<Node>,
  },

  Identifier {
    line: usize,
    value: String,
    inferred_type: NodeType,
  },

  IntLiteral {
    line: usize,
    value: String,
    inferred_type: NodeType,
  },

  FloatLiteral {
    line: usize,
    value: String,
    inferred_type: NodeType,
  },

  StringLiteral {
    line_start: usize,
    line_end: usize,
    value: String,
    inferred_type: NodeType,
  },

  StringInterpolation {
    line_start: usize,
    line_end: usize,
    parts: Vec<Node>,
    inferred_type: NodeType,
  },

  Assignment {
    line_start: usize,
    line_end: usize,
    is_constant: bool,
    left: Box<Node>,
    right: Box<Node>,
    inferred_type: NodeType,
  },

  ArrayLiteral {
    line_start: usize,
    line_end: usize,
    elements: Vec<Node>,
    inferred_type: NodeType,
  },

  DictLiteral {
    line_start: usize,
    line_end: usize,
    entries: Vec<Node>,
    inferred_type: NodeType,
  },

  DictEntry {
    line_start: usize,
    line_end: usize,
    key: Box<Node>,
    value: Box<Node>,
  },
}

#[derive(Debug)]
pub enum NodeType {
  Unknown,
  Func {
    param_types: Vec<NodeType>,
    return_type: Box<NodeType>,
  },
}
