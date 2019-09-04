#[derive(Debug, Clone)]
pub enum UnaryOperator {
  Minus,
}

#[derive(Debug, Clone)]
pub enum NodeType {
  Unknown,
}

#[derive(Debug, Clone)]
pub enum Node {
  Module {
    body: Vec<Node>,
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

  Block {
    line_start: usize,
    col_start: usize,
    line_end: usize,
    col_end: usize,
    params: Vec<Node>,
    body: Vec<Node>,
    inferred_type: NodeType,
  },

  Call {
    line_start: usize,
    col_start: usize,
    line_end: usize,
    col_end: usize,
    callee: Box<Node>,
    arguments: Vec<Node>,
    inferred_type: NodeType,
  },

  // a.b.c -> (obj:(a.b) , prop:c)
  Chain {
    line_start: usize,
    col_start: usize,
    line_end: usize,
    col_end: usize,
    object: Box<Node>,
    property: Box<Node>,
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

  Tuple {
    line_start: usize,
    line_end: usize,
    col_start: usize,
    col_end: usize,
    entries: Vec<Node>,
    inferred_type: NodeType,
  },

  UnaryOperation {
    line_start: usize,
    line_end: usize,
    col_start: usize,
    col_end: usize,
    left_side: Box<Node>,
    right_side: Box<Node>,
    operator: UnaryOperator,
    inferred_type: NodeType,
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