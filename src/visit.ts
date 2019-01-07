export abstract class Visitor {
  private readonly ast: AstNode;

  constructor(ast: AstNode) {
    this.ast = ast;
  }

  visitAssignmentExpression(node: AssignmentExpression) {}

  visitBinaryExpression(node: BinaryExpression) {}

  visitBlock(node: Block) {}

  visitBooleanLiteral(node: BooleanLiteral) {}

  visitCallExpression(node: CallExpression) {}

  visitCharLiteral(node: CharLiteral) {}

  visitDictEntry(node: DictExpression) {}

  visitDictExpression(node: DictExpression) {}

  visitIdentifier(node: Identifier) {}

  visitInterpolatedStringLiteral(node: InterpolatedStringLiteral) {}

  visitListExpression(node: ListExpression) {}

  visitMethodDefinition(node: MethodDefinition) {}

  visitModule(node: Module) {}

  visitNumericLiteral(node: NumericLiteral) {}

  visitOperator(node: Operator) {}

  visitStringLiteral(node: StringLiteral) {}

  visitTypeDefinition(node: TypeDefinition) {}

  visitTypeExpression(node: TypeExpression) {}

  visitTypeIdentifier(node: TypeIdentifier) {}

  visit(node: AstNode): void {}
}
