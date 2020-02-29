pub type StartOffset = usize;
pub type EndOffset = usize;
pub type Position = (StartOffset, EndOffset);
pub type NodeId = usize;
pub type SignaturePart = (Box<IdentNode>, Vec<TypeNode>);
pub type Signature = Vec<SignaturePart>;

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
  TypeDef(TypeDefNode),
  Def(DefNode),
  Expr(ExprNode),
}

#[derive(Debug)]
pub struct TypeDefNode {
  pub id: NodeId,
  pub pos: Position,
  pub kind: TypeDefKind,
  pub name: Box<IdentNode>,
  pub generics: Vec<IdentNode>,
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
pub enum TypeDefKind {
  // alias StringList = List(String)
  Alias {
    of: TypeNode,
  },
  // enum Color = | Red | Green | Blue
  Enum {
    variants: Vec<TypeNode>,
  },
  // struct Person = (name :: String, age :: Int)
  Struct {
    fields: Vec<(IdentNode, TypeNode)>,
  },
  // trait Named = .name :: String .getName() -> String
  Trait {
    fields: Vec<(IdentNode, TypeNode)>,
    signatures: Vec<Signature>,
  },
}

#[derive(Debug)]
pub enum DefKind {
  // def hi(A, B) -> Ret { ... }
  Function {
    signature: Signature,
  },
  // def (Receiver).hi() -> Ret { ... }
  Method {
    receiver: Box<TypeNode>,
    signature: Signature,
  },
  // def (Receiver)[Int] -> Ret { ... }
  Index {
    receiver: Box<TypeNode>,
    index: Box<TypeNode>,
  },
  // def (A) ++ (B) -> Ret
  BinaryOperator {
    left: Box<TypeNode>,
    op: Box<OperatorNode>,
    right: Box<TypeNode>,
  },
  // def ~(A) -> Ret
  UnaryOperator {
    op: Box<OperatorNode>,
    right: Box<TypeNode>,
  },
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
  Array(Vec<ExprNode>),
  Assignment {
    left: Box<IdentNode>,
    right: Box<ExprNode>,
  },
  BinaryOperation {
    left: Box<ExprNode>,
    op: Box<OperatorNode>,
    right: Box<ExprNode>,
  },
  Block {
    params: Vec<IdentNode>,
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
  Identifier(IdentNode),
  Index(Box<ExprNode>, Box<ExprNode>),
  Interpolation(Vec<ExprNode>),
  Literal(LitNode),
  Match(MatchNode),
  Tuple(Vec<ExprNode>),
  UnaryOperation {
    op: Box<OperatorNode>,
    right: Box<ExprNode>,
  },
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
  FloatDecimal(f64),
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
  UnexpectedDictValueInArray,
  UnexpectedEOF,
  UnexpectedToken,
  UnclosedParentheses,
  MissingIdentifier,
  MissingIndexBetweenBrackets,
  MissingDefinitionBody,
  MissingDictValue,
  MissingEnumValues,
  MissingExpressionAfterDot,
  MissingExpressionAfterOperator,
  MissingMatchCases,
  MissingReturnType,
  MissingStructFields,
  MissingType,
}
