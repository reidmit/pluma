use crate::ast::*;
use crate::diagnostic::*;
use crate::errors::*;
use crate::module::Module;
use crate::scope::*;
use crate::value_type::*;

pub struct Analyzer<'compiler> {
  diagnostics: &'compiler mut Vec<Diagnostic>,
  scope: Scope,
}

impl<'compiler> Analyzer<'compiler> {
  pub fn new(diagnostics: &'compiler mut Vec<Diagnostic>) -> Self {
    Self {
      diagnostics,
      scope: Scope::new(),
    }
  }

  pub fn analyze(&mut self, module: &mut Module) {
    if let Some(ast) = &mut module.ast {
      self.scope.enter();

      for definition in &mut ast.body {
        self.analyze_definition(definition)
      }

      let _ = self.scope.exit();
    }
  }

  fn analyze_definition(&mut self, definition: &mut DefinitionNode) {
    println!("analyzing def: {:#?}", definition);

    let name = definition.name.name.clone();

    let resolved_type = match &mut definition.kind {
      DefinitionKind::Expr(expr) => self.analyze_expr(expr),
    };

    if let ValueType::Unknown = resolved_type {
      self.error(
        definition.name.pos,
        AnalysisErrorKind::CouldNotInferDefinitionType { name: name.clone() },
      );
    }

    self
      .scope
      .add_binding(BindingKind::Def, name, resolved_type, definition.name.pos)
  }

  fn analyze_expr(&mut self, expr: &mut ExprNode) -> ValueType {
    match &mut expr.kind {
      ExprKind::Literal(literal) => self.analyze_literal(literal),
      _ => ValueType::Unknown,
    }
  }

  fn analyze_literal(&mut self, literal: &mut LiteralNode) -> ValueType {
    match &mut literal.kind {
      LiteralKind::IntDecimal(..) => ValueType::Int,
      LiteralKind::IntBinary(..) => ValueType::Int,
      LiteralKind::IntOctal(..) => ValueType::Int,
      LiteralKind::IntHex(..) => ValueType::Int,
      LiteralKind::FloatDecimal(..) => ValueType::Float,
      LiteralKind::Str(..) => ValueType::String,
    }
  }

  fn warning(&mut self, pos: (usize, usize), kind: AnalysisErrorKind) {
    self
      .diagnostics
      .push(Diagnostic::warning(AnalysisError { pos, kind }))
  }

  fn error(&mut self, pos: (usize, usize), kind: AnalysisErrorKind) {
    self
      .diagnostics
      .push(Diagnostic::error(AnalysisError { pos, kind }))
  }
}
