#[derive(Debug, Clone)]
pub enum UnaryOperator {
  Minus,
}

#[derive(Debug, Clone)]
pub enum NodeType {
  Unknown,
}

#[derive(Debug, Clone)]
pub enum NumericValue {
  Int(i64),
  Float(f64)
}

#[derive(Debug, Clone)]
pub enum Node {
  Module {
    body: Vec<Node>,
  },

  Assignment {
    start: usize,
    end: usize,
    is_constant: bool,
    left: Box<Node>,
    right: Box<Node>,
    inferred_type: NodeType,
  },

  Block {
    start: usize,
    end: usize,
    params: Vec<Node>,
    body: Vec<Node>,
    inferred_type: NodeType,
  },

  Call {
    start: usize,
    end: usize,
    callee: Box<Node>,
    arguments: Vec<Node>,
    inferred_type: NodeType,
  },

  Chain {
    start: usize,
    end: usize,
    object: Box<Node>,
    property: Box<Node>,
  },

  Grouping {
    start: usize,
    end: usize,
    expr: Box<Node>,
    inferred_type: NodeType,
  },

  Identifier {
    start: usize,
    end: usize,
    name: String,
    inferred_type: NodeType,
  },

  NumericLiteral {
    start: usize,
    end: usize,
    value: NumericValue,
    raw_value: String,
    inferred_type: NodeType,
  },

  StringInterpolation {
    start: usize,
    end: usize,
    parts: Vec<Node>,
    inferred_type: NodeType,
  },

  Tuple {
    start: usize,
    end: usize,
    entries: Vec<Node>,
    inferred_type: NodeType,
  },

  UnaryOperation {
    start: usize,
    end: usize,
    left_side: Box<Node>,
    right_side: Box<Node>,
    operator: UnaryOperator,
    inferred_type: NodeType,
  },
}

pub fn extract_location(node: &Node) -> (usize, usize) {
  match node {
    Node::Assignment { start, end, .. } => (*start, *end),
    Node::Block { start, end, .. } => (*start, *end),
    Node::Call { start, end, .. } => (*start, *end),
    Node::Chain { start, end, .. } => (*start, *end),
    Node::Grouping { start, end, .. } => (*start, *end),
    Node::Identifier { start, end, .. } => (*start, *end),
    Node::NumericLiteral { start, end, .. } => (*start, *end),
    Node::StringInterpolation { start, end, .. } => (*start, *end),
    Node::Tuple { start, end, .. } => (*start, *end),
    Node::UnaryOperation { start, end, .. } => (*start, *end),

    something_else => unimplemented!("unexpected node: {:#?}", something_else),
  }
}