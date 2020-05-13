use crate::types::ValueType;
use uuid::Uuid;

pub type Position = (usize, usize);
pub type NodeId = Uuid;
pub type SignaturePart = (Box<IdentifierNode>, Vec<TypeExprNode>);
pub type Signature = Vec<SignaturePart>;

#[derive(Debug)]
pub struct DefNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: DefKind,
  pub return_type: Option<TypeExprNode>,
  pub params: Vec<PatternNode>,
  pub body: Vec<StatementNode>,
}

#[derive(Debug)]
pub enum DefKind {
  // def hi(A, B) -> Ret { ... }
  Function {
    signature: Signature,
  },
  // def (Receiver).hi() -> Ret { ... }
  Method {
    receiver: Box<TypeExprNode>,
    signature: Signature,
  },
  // def (Receiver)[Int] -> Ret { ... }
  Index {
    receiver: Box<TypeExprNode>,
    index: Box<TypeExprNode>,
  },
  // def (A) ++ (B) -> Ret { ... }
  BinaryOperator {
    left: Box<TypeExprNode>,
    op: Box<OperatorNode>,
    right: Box<TypeExprNode>,
  },
  // def ~(A) -> Ret { ... }
  UnaryOperator {
    op: Box<OperatorNode>,
    right: Box<TypeExprNode>,
  },
}

#[derive(Debug)]
pub struct ExprNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: ExprKind,
  pub typ: Option<ValueType>,
}

#[derive(Debug)]
pub enum ExprKind {
  Array(Vec<ExprNode>),
  Assignment {
    left: Box<IdentifierNode>,
    right: Box<ExprNode>,
  },
  BinaryOperation {
    left: Box<ExprNode>,
    op: Box<OperatorNode>,
    right: Box<ExprNode>,
  },
  Block {
    params: Vec<PatternNode>,
    body: Vec<StatementNode>,
  },
  Call {
    callee: Box<ExprNode>,
    args: Vec<ExprNode>,
  },
  Chain {
    obj: Box<ExprNode>,
    prop: Box<ExprNode>,
  },
  Dict(Vec<(ExprNode, ExprNode)>),
  EmptyTuple,
  Grouping(Box<ExprNode>),
  Identifier(IdentifierNode),
  Index(Box<ExprNode>, Box<ExprNode>),
  Interpolation(Vec<ExprNode>),
  Literal(LiteralNode),
  Match(MatchNode),
  Tuple(Vec<ExprNode>),
  UnaryOperation {
    op: Box<OperatorNode>,
    right: Box<ExprNode>,
  },
  Underscore,
}

#[derive(Debug, Clone)]
pub struct IdentifierNode {
  pub id: NodeId,
  pub pos: Position,
  pub name: String,
  pub typ: Option<ValueType>,
}

#[derive(Debug)]
pub struct LetNode {
  pub id: NodeId,
  pub pos: Position,
  pub pattern: PatternNode,
  pub value: ExprNode,
}

#[derive(Debug)]
pub struct LiteralNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: LiteralKind,
  pub typ: Option<ValueType>,
}

#[derive(Debug)]
pub enum LiteralKind {
  FloatDecimal(f64),
  IntDecimal(i128),
  IntOctal(i128),
  IntHex(i128),
  IntBinary(i128),
  Str(String),
}

#[derive(Debug)]
pub struct MatchNode {
  pub id: NodeId,
  pub pos: Position,
  pub subject: Box<ExprNode>,
  pub cases: Vec<MatchCaseNode>,
}

#[derive(Debug)]
pub struct MatchCaseNode {
  pub id: NodeId,
  pub pos: Position,
  pub pattern: PatternNode,
  pub body: ExprNode,
}

#[derive(Debug)]
pub struct ModuleNode {
  pub id: NodeId,
  pub pos: Position,
  pub body: Vec<TopLevelStatementNode>,
}

#[derive(Debug)]
pub struct OperatorNode {
  pub id: NodeId,
  pub pos: Position,
  pub name: String,
}

#[derive(Debug)]
pub struct PatternNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: PatternKind,
}

#[derive(Debug)]
pub enum PatternKind {
  Ident(IdentifierNode),
}

#[derive(Debug)]
pub struct ReturnNode {
  pub id: NodeId,
  pub pos: Position,
  pub value: ExprNode,
}

#[derive(Debug)]
pub struct StatementNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: StatementKind,
}

#[derive(Debug)]
pub enum StatementKind {
  Let(LetNode),
  Expr(ExprNode),
  Return(ReturnNode),
}

#[derive(Debug)]
pub struct TopLevelStatementNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: TopLevelStatementKind,
}

#[derive(Debug)]
pub enum TopLevelStatementKind {
  Let(LetNode),
  TypeDef(TypeDefNode),
  Def(DefNode),
  Expr(ExprNode),
}

#[derive(Debug)]
pub struct TypeExprNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: TypeExprKind,
}

#[derive(Debug)]
pub enum TypeExprKind {
  // e.g. String
  Ident(IdentifierNode),
  // e.g. List(String),
  Generic(IdentifierNode, Vec<TypeExprNode>),
  // e.g. { String -> Bool }
  Block(Vec<TypeExprNode>, Box<TypeExprNode>),
  // e.g. (String, Bool)
  Tuple(Vec<TypeExprNode>),
}

#[derive(Debug)]
pub struct TypeDefNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: TypeDefKind,
  pub name: Box<IdentifierNode>,
  pub generics: Vec<IdentifierNode>,
}

#[derive(Debug)]
pub enum TypeDefKind {
  // alias StringList = List(String)
  Alias {
    of: TypeExprNode,
  },
  // enum Color | Red | Green | Blue
  Enum {
    variants: Vec<TypeExprNode>,
  },
  // struct Person (name :: String, age :: Int)
  Struct {
    fields: Vec<(IdentifierNode, TypeExprNode)>,
  },
  // trait Named .name :: String .getName() -> String
  Trait {
    fields: Vec<(IdentifierNode, TypeExprNode)>,
    methods: Vec<Signature>,
  },
}

#[derive(Debug, Clone)]
pub struct UseNode {
  pub id: NodeId,
  pub pos: Position,
  pub module_name: String,
  pub qualifier: Box<IdentifierNode>,
}
