use crate::analysis_error::{AnalysisError, AnalysisErrorKind};
use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::scope::{Binding, Scope};
use crate::types::ValueType;
use crate::visitor_mut::VisitorMut;
use std::collections::HashMap;
use uuid::Uuid;

pub struct Analyzer<'a> {
  pub diagnostics: Vec<Diagnostic>,
  scope: &'a mut Scope,
}

impl<'a> Analyzer<'a> {
  pub fn new(scope: &'a mut Scope) -> Analyzer<'a> {
    Analyzer {
      scope,
      diagnostics: Vec::new(),
    }
  }

  fn error(&mut self, err: AnalysisError) {
    let pos = err.pos;
    self.diagnostic(Diagnostic::error(err).with_pos(pos))
  }

  fn warning(&mut self, err: AnalysisError) {
    let pos = err.pos;
    self.diagnostic(Diagnostic::warning(err).with_pos(pos))
  }

  fn diagnostic(&mut self, diag: Diagnostic) {
    self.diagnostics.push(diag)
  }
}

impl<'a> VisitorMut for Analyzer<'a> {
  fn leave_let(&mut self, node: &mut LetNode) {
    match &mut node.pattern.kind {
      PatternKind::Ident(ident_node) => {
        let existing_binding = self.scope.get_let_binding(&ident_node.name);

        if existing_binding.is_some() {
          self.error(AnalysisError {
            pos: ident_node.pos,
            kind: AnalysisErrorKind::NameAlreadyInScope(ident_node.name.clone()),
          })
        } else {
          if let Some(value_type) = &node.value.typ {
            self
              .scope
              .add_let_binding(ident_node.name.clone(), value_type.clone(), ident_node.pos);

            ident_node.typ = Some(value_type.clone());
          }
        }
      }
    }
  }

  fn leave_literal(&mut self, node: &mut LiteralNode) {
    match &node.kind {
      LiteralKind::IntDecimal { .. } => node.typ = Some(ValueType::Named("Int".to_owned())),
      LiteralKind::IntBinary { .. } => node.typ = Some(ValueType::Named("Int".to_owned())),
      LiteralKind::IntHex { .. } => node.typ = Some(ValueType::Named("Int".to_owned())),
      LiteralKind::IntOctal { .. } => node.typ = Some(ValueType::Named("Int".to_owned())),
      LiteralKind::FloatDecimal { .. } => node.typ = Some(ValueType::Named("Float".to_owned())),
      LiteralKind::Str { .. } => node.typ = Some(ValueType::Named("String".to_owned())),
    }
  }

  fn enter_expr(&mut self, node: &mut ExprNode) {
    match &node.kind {
      ExprKind::Block { params, body } => {
        self.scope.enter();

        for param in params {
          self
            .scope
            .add_let_binding(param.name.clone(), ValueType::Unknown, param.pos);
        }
      }

      _ => {}
    }
  }

  fn leave_expr(&mut self, node: &mut ExprNode) {
    match &node.kind {
      ExprKind::Identifier(ident_node) => node.typ = ident_node.typ.clone(),

      ExprKind::Literal(lit_node) => node.typ = lit_node.typ.clone(),

      ExprKind::Assignment { left, right } => {
        let existing_binding = self.scope.get_let_binding(&left.name);

        if let Some(binding) = existing_binding {
          let current_type = binding.clone();
          let new_type = right.typ.as_ref().unwrap().clone();

          if current_type != new_type {
            self.error(AnalysisError {
              pos: right.pos,
              kind: AnalysisErrorKind::ReassignmentTypeMismatch {
                expected: current_type,
                actual: new_type,
              },
            })
          }
        }
      }

      ExprKind::Block { params, body } => {
        let mut param_types = Vec::new();
        let mut return_type = ValueType::Nothing;

        for param in params {
          param_types.push(ValueType::Unknown);
        }

        for stmt in body {
          if let StatementKind::Expr(expr) = &stmt.kind {
            if let Some(typ) = &expr.typ {
              return_type = typ.clone();
            }
          }
        }

        node.typ = Some(ValueType::Func(param_types, Box::new(return_type)));

        if let Err(diagnostics) = self.scope.exit() {
          for diagnostic in diagnostics {
            self.diagnostic(diagnostic);
          }
        }
      }

      ExprKind::Call { callee, args } => {
        let callee_type = callee.typ.as_ref().unwrap();

        match callee_type {
          ValueType::Func(param_types, return_type) => {
            // TODO assert on matching param types

            node.typ = Some(*return_type.clone());
          }
          _ => self.error(AnalysisError {
            pos: node.pos,
            kind: AnalysisErrorKind::CalleeNotCallable(callee_type.clone()),
          }),
        }
      }

      t => todo!("more expr kinds: {:#?}", t),
    }
  }

  fn enter_identifier(&mut self, node: &mut IdentifierNode) {
    match self.scope.get_let_binding(&node.name) {
      Some(typ) => node.typ = Some(typ.clone()),
      None => self.error(AnalysisError {
        pos: node.pos,
        kind: AnalysisErrorKind::UndefinedVariable(node.name.clone()),
      }),
    };
  }
}
