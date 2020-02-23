use crate::tokens::Token;

pub type StartOffset = usize;
pub type EndOffset = usize;
pub type Position = (StartOffset, EndOffset);
pub type NodeId = usize;

#[derive(Debug)]
pub struct ModuleNode {
  pub id: NodeId,
  pub pos: Position,
  pub body: Vec<TopLevelStatementNode>,
}

#[derive(Debug)]
pub struct TopLevelStatementNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: TopLevelStatementKind,
}

#[derive(Debug)]
pub enum TopLevelStatementKind {
  // UseStatement(UseStatementNode),
  // private
  Let(LetNode),
  // TypeDef(TypeDefNode),
  Def(DefNode),
  Expr(ExprNode),
}

#[derive(Debug)]
pub struct DefNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: DefKind,
  pub return_type: Option<TypeNode>,
  pub params: Vec<IdentNode>,
  pub body: Vec<StatementNode>,
}

#[derive(Debug)]
pub enum DefKind {
  // def hi(A, B) -> Ret { ... }
  Function(Vec<(Box<IdentNode>, Vec<TypeNode>)>),
  // def (Receiver).hi() -> Ret { ... }
  Method(Box<TypeNode>, Vec<(Box<IdentNode>, Vec<TypeNode>)>),
  // def (Receiver)[Int] -> Ret { ... }
  Index(Box<TypeNode>, Box<TypeNode>),
  // def (A) ++ (B) -> Ret
  BinaryOperator(Box<TypeNode>, Box<OperatorNode>, Box<TypeNode>),
  // def ~(A) -> Ret
  UnaryOperator(Box<OperatorNode>, Box<TypeNode>),
}

#[derive(Debug)]
pub struct OperatorNode {
  pub id: NodeId,
  pub pos: Position,
  pub name: String,
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
}

#[derive(Debug)]
pub struct LetNode {
  pub id: NodeId,
  pub pos: Position,
  pub pattern: PatternNode,
  pub value: ExprNode,
}

#[derive(Debug)]
pub struct ExprNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: ExprKind,
}

#[derive(Debug)]
pub enum ExprKind {
  Assignment(Box<IdentNode>, Box<ExprNode>),
  BinaryOperation(Box<ExprNode>, Box<OperatorNode>, Box<ExprNode>),
  Block(Vec<IdentNode>, Vec<StatementNode>),
  EmptyTuple,
  Grouping(Box<ExprNode>),
  Identifier(IdentNode),
  Interpolation(Vec<ExprNode>),
  Literal(LitNode),
  Match(MatchNode),
  Tuple(Vec<ExprNode>),
  UnaryOperation(Box<OperatorNode>, Box<ExprNode>),
}

#[derive(Debug)]
pub struct IdentNode {
  pub id: NodeId,
  pub pos: Position,
  pub name: String,
}

#[derive(Debug)]
pub struct LitNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: LitKind,
}

#[derive(Debug)]
pub enum LitKind {
  IntDecimal(i128),
  IntOctal(i128),
  IntHex(i128),
  IntBinary(i128),
  Str(String),
}

#[derive(Debug)]
pub struct PatternNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: PatternKind,
}

#[derive(Debug)]
pub enum PatternKind {
  Ident(IdentNode),
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
pub struct TypeNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: TypeKind,
}

#[derive(Debug)]
pub enum TypeKind {
  // e.g. String
  Ident(IdentNode),
  // e.g. List(String),
  Generic(IdentNode, Vec<TypeNode>),
  // e.g. { String -> Bool }
  Block(Vec<TypeNode>, Box<TypeNode>),
  // e.g. (String, Bool)
  Tuple(Vec<TypeNode>),
}

#[derive(Debug, Copy, Clone)]
pub struct ParseError {
  pub pos: Position,
  pub kind: ParseErrorKind,
}

#[derive(Debug, Copy, Clone)]
pub enum ParseErrorKind {
  UnexpectedToken(Token),
  UnclosedParentheses,
  MissingIdentifier,
  MissingDefinitionBody,
  MissingReturnType,
  MissingType,
  MissingExpressionAfterOperator,
}
