use crate::analysis_error::{AnalysisError, AnalysisErrorKind};
use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::scope::{Binding, Scope};
use crate::types::ValueType;
use crate::visitor::Visitor;
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

  fn destructure_pattern(&mut self, pattern: &mut PatternNode, typ: &mut ValueType) {
    match &mut pattern.kind {
      PatternKind::Identifier(ident_node) => {
        let existing_binding = self.scope.get_let_binding(&ident_node.name);

        if existing_binding.is_some() {
          self.error(AnalysisError {
            pos: ident_node.pos,
            kind: AnalysisErrorKind::NameAlreadyInScope(ident_node.name.clone()),
          })
        } else {
          self
            .scope
            .add_let_binding(ident_node.name.clone(), typ.clone(), ident_node.pos);

          ident_node.typ = typ.clone();
        }
      }

      PatternKind::Tuple(element_patterns) => match typ {
        ValueType::Tuple(element_types) => {
          if element_patterns.len() != element_types.len() {
            return self.error(AnalysisError {
              pos: pattern.pos,
              kind: AnalysisErrorKind::PatternMismatchTupleSize {
                pattern_size: element_patterns.len(),
                value_size: element_types.len(),
              },
            });
          }

          for i in 0..element_patterns.len() {
            let mut element_pattern = element_patterns.get_mut(i).unwrap();
            let mut element_type = element_types.get_mut(i).unwrap();

            self.destructure_pattern(&mut element_pattern, &mut element_type);
          }
        }

        typ => self.error(AnalysisError {
          pos: pattern.pos,
          kind: AnalysisErrorKind::PatternMismatchExpectedTuple(typ.clone()),
        }),
      },

      PatternKind::Underscore => {}

      PatternKind::Literal(lit_node) => self.error(AnalysisError {
        pos: pattern.pos,
        kind: AnalysisErrorKind::CannotAssignToLiteral,
      }),

      _ => todo!("other pattern kinds"),
    }
  }
}

impl<'a> Visitor for Analyzer<'a> {
  fn leave_module(&mut self, node: &mut ModuleNode) {
    println!("end scope: {:#?}", self.scope);
  }

  fn leave_call(&mut self, node: &mut CallNode) {
    let callee_type = &node.callee.typ;

    match callee_type {
      ValueType::Func(param_types, return_type) => {
        // TODO assert on matching param types

        node.typ = *return_type.clone();
      }
      _ => self.error(AnalysisError {
        pos: node.pos,
        kind: AnalysisErrorKind::CalleeNotCallable(callee_type.clone()),
      }),
    }
  }

  fn leave_let(&mut self, node: &mut LetNode) {
    self.destructure_pattern(&mut node.pattern, &mut node.value.typ)
  }

  fn leave_literal(&mut self, node: &mut LiteralNode) {
    match &node.kind {
      LiteralKind::IntDecimal { .. } => node.typ = ValueType::Named("Int".to_owned()),
      LiteralKind::IntBinary { .. } => node.typ = ValueType::Named("Int".to_owned()),
      LiteralKind::IntHex { .. } => node.typ = ValueType::Named("Int".to_owned()),
      LiteralKind::IntOctal { .. } => node.typ = ValueType::Named("Int".to_owned()),
      LiteralKind::FloatDecimal { .. } => node.typ = ValueType::Named("Float".to_owned()),
      LiteralKind::Str { .. } => node.typ = ValueType::Named("String".to_owned()),
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

      ExprKind::Call(call_node) => node.typ = call_node.typ.clone(),

      ExprKind::Assignment { left, right } => {
        let existing_binding = self.scope.get_let_binding(&left.name);

        if let Some(binding) = existing_binding {
          let current_type = binding.clone();
          let new_type = right.typ.clone();

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
            return_type = expr.typ.clone();
          }
        }

        node.typ = ValueType::Func(param_types, Box::new(return_type));

        if let Err(diagnostics) = self.scope.exit() {
          for diagnostic in diagnostics {
            self.diagnostic(diagnostic);
          }
        }
      }

      ExprKind::Tuple(elements) => {
        let mut element_types = Vec::new();

        for element in elements {
          element_types.push(element.typ.clone());
        }

        node.typ = ValueType::Tuple(element_types);
      }

      ExprKind::Interpolation(parts) => {
        let string_type = ValueType::Named("String".to_owned());

        for part in parts {
          if part.typ != string_type {
            self.error(AnalysisError {
              pos: part.pos,
              kind: AnalysisErrorKind::TypeMismatchInStringInterpolation(part.typ.clone()),
            })
          }
        }

        node.typ = string_type;
      }

      ExprKind::EmptyTuple => node.typ = ValueType::Nothing,

      t => todo!("more expr kinds: {:#?}", t),
    }
  }

  fn enter_identifier(&mut self, node: &mut IdentifierNode) {
    match self.scope.get_let_binding(&node.name) {
      Some(typ) => node.typ = typ.clone(),
      None => self.error(AnalysisError {
        pos: node.pos,
        kind: AnalysisErrorKind::UndefinedVariable(node.name.clone()),
      }),
    };
  }
}
