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
  pattern_depth: usize,
}

impl<'a> Analyzer<'a> {
  pub fn new(scope: &'a mut Scope) -> Analyzer<'a> {
    Analyzer {
      scope,
      diagnostics: Vec::new(),
      pattern_depth: 0,
    }
  }

  fn error(&mut self, err: AnalysisError) {
    let pos = err.pos;
    self.diagnostics.push(Diagnostic::error(err).with_pos(pos))
  }

  fn warning(&mut self, err: AnalysisError) {
    let pos = err.pos;
    self
      .diagnostics
      .push(Diagnostic::warning(err).with_pos(pos))
  }
}

impl<'a> VisitorMut for Analyzer<'a> {
  fn enter_module(&mut self, node: &mut ModuleNode) {
    // self.push_scope();

    // for (name, typ) in &self.top_level_defs {
    //   // self.add_binding(name.clone(), (0, 0), node_id: Uuid)
    // }
  }

  fn leave_module(&mut self, node: &mut ModuleNode) {
    // self.pop_scope();
  }

  fn leave_let(&mut self, node: &mut LetNode) {
    println!("leaving let!");

    match &mut node.pattern.kind {
      PatternKind::Ident(ident_node) => {
        let existing_binding = self.scope.get_let_binding(&ident_node.name);

        if existing_binding.is_some() {
          self.error(AnalysisError {
            pos: ident_node.pos,
            kind: AnalysisErrorKind::NameAlreadyInScope(ident_node.name.clone()),
          })
        } else {
          let value_type = node.value.typ.as_ref().expect("no type for let value");

          self
            .scope
            .add_let_binding(ident_node.name.clone(), value_type.clone());

          ident_node.typ = Some(value_type.clone());
        }
      }
    }
  }

  fn leave_literal(&mut self, node: &mut LiteralNode) {
    match &node.kind {
      LiteralKind::IntDecimal { .. } => node.typ = Some(ValueType::CoreInt),
      LiteralKind::IntBinary { .. } => node.typ = Some(ValueType::CoreInt),
      LiteralKind::IntHex { .. } => node.typ = Some(ValueType::CoreInt),
      LiteralKind::IntOctal { .. } => node.typ = Some(ValueType::CoreInt),
      LiteralKind::FloatDecimal { .. } => node.typ = Some(ValueType::CoreFloat),
      LiteralKind::Str { .. } => node.typ = Some(ValueType::CoreString),
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

      _ => todo!("more expr kinds"),
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
