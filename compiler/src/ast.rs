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
    start: usize,
    end: usize,
    imports: Vec<Node>,
    body: Vec<Node>,
  },

  Array {
    start: usize,
    end: usize,
    elements: Vec<Node>,
    inferred_type: NodeType,
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

  Dict {
    start: usize,
    end: usize,
    entries: Vec<Node>,
    inferred_type: NodeType,
  },

  DictEntry {
    start: usize,
    end: usize,
    key: Box<Node>,
    value: Box<Node>,
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

  Import {
    start: usize,
    end: usize,
    alias: Option<String>,
    path: String,
  },

  Match {
    start: usize,
    end: usize,
    discriminant: Box<Node>,
    cases: Vec<Node>,
    inferred_type: NodeType,
  },

  MatchCase {
    start: usize,
    end: usize,
    pattern: Box<Node>,
    body: Box<Node>,
    inferred_type: NodeType,
  },

  NumericLiteral {
    start: usize,
    end: usize,
    value: NumericValue,
    raw_value: String,
    inferred_type: NodeType,
  },

  Reassignment {
    start: usize,
    end: usize,
    left: Box<Node>,
    right: Box<Node>,
    inferred_type: NodeType,
  },

  StringInterpolation {
    start: usize,
    end: usize,
    parts: Vec<Node>,
    inferred_type: NodeType,
  },

  StringLiteral {
    start: usize,
    end: usize,
    value: String,
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
    operator: UnaryOperator,
    expr: Box<Node>,
    inferred_type: NodeType,
  },
}

pub fn get_node_location(node: &Node) -> (usize, usize) {
  match node {
    Node::Array { start, end, .. } => (*start, *end),
    Node::Assignment { start, end, .. } => (*start, *end),
    Node::Block { start, end, .. } => (*start, *end),
    Node::Call { start, end, .. } => (*start, *end),
    Node::Chain { start, end, .. } => (*start, *end),
    Node::Dict { start, end, .. } => (*start, *end),
    Node::DictEntry { start, end, .. } => (*start, *end),
    Node::Grouping { start, end, .. } => (*start, *end),
    Node::Identifier { start, end, .. } => (*start, *end),
    Node::Import { start, end, .. } => (*start, *end),
    Node::Match { start, end, .. } => (*start, *end),
    Node::MatchCase { start, end, .. } => (*start, *end),
    Node::Module { start, end, .. } => (*start, *end),
    Node::NumericLiteral { start, end, .. } => (*start, *end),
    Node::Reassignment { start, end, .. } => (*start, *end),
    Node::StringInterpolation { start, end, .. } => (*start, *end),
    Node::StringLiteral { start, end, .. } => (*start, *end),
    Node::Tuple { start, end, .. } => (*start, *end),
    Node::UnaryOperation { start, end, .. } => (*start, *end),
  }
}