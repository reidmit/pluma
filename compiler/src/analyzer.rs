use crate::analysis_error::{AnalysisError, AnalysisErrorKind};
use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::types::Type;
use crate::visitor::Visitor;
use std::collections::HashMap;
use uuid::Uuid;

pub struct Analyzer {
  pub diagnostics: Vec<Diagnostic>,
  scopes: Vec<Scope>,
  node_types: HashMap<Uuid, Type>,
}

impl Analyzer {
  pub fn new() -> Analyzer {
    Analyzer {
      diagnostics: Vec::new(),
      scopes: Vec::new(),
      node_types: HashMap::new(),
    }
  }

  fn push_scope(&mut self) {
    self.scopes.push(Scope::new());
  }

  fn pop_scope(&mut self) {
    let popped_scope = self.scopes.pop().unwrap();

    for (name, binding) in popped_scope.bindings.iter() {
      if binding.references == 0 {
        self.warning(AnalysisError {
          pos: binding.pos,
          kind: AnalysisErrorKind::UnusedVariable(name.to_string()),
        })
      }
    }
  }

  fn add_node_type(&mut self, node_id: Uuid, node_type: Type) {
    self.node_types.insert(node_id, node_type);
  }

  fn add_binding(&mut self, name: String, pos: (usize, usize), node_id: Uuid) {
    let current_scope = self.scopes.last_mut().unwrap();

    current_scope.add_binding(name, node_id, pos, false);
  }

  fn add_reference(&mut self, name: &String) {
    let current_scope = self.scopes.last_mut().unwrap();

    current_scope.add_reference(name);
  }

  fn get_binding_type(&mut self, node: &IdentifierNode) -> Result<Type, AnalysisError> {
    let current_scope = self.scopes.last().expect("should always have a scope");

    match current_scope.bindings.get(&node.name) {
      Some(binding) => {
        let t = self.node_types.get(&binding.node_id).unwrap().clone();
        self.add_reference(&node.name);
        Ok(t)
      }
      None => Err(AnalysisError {
        pos: node.pos,
        kind: AnalysisErrorKind::UndefinedVariable(node.name.clone()),
      }),
    }
  }

  fn get_node_type(&self, node_id: &Uuid) -> Result<Type, AnalysisError> {
    match self.node_types.get(node_id) {
      Some(node_type) => Ok(node_type.clone()),

      None => unreachable!("no type for node {} (scopes: {:#?})", node_id, self.scopes),
    }
  }

  fn get_expr_type(&mut self, node: &ExprNode) -> Result<Type, AnalysisError> {
    match &node.kind {
      ExprKind::Block { params, body } => {
        self.push_scope();

        let mut param_types = vec![];

        for param in params {
          param_types.push(Type::Unknown)
        }

        let mut return_type = Type::Nothing;

        for stmt in body {
          if let StatementKind::Expr(expr_stmt) = &stmt.kind {
            let node_type = self.get_expr_type(&expr_stmt)?;
            return_type = node_type;
          }
        }

        self.pop_scope();

        Ok(Type::Func(param_types, Box::new(return_type)))
      }

      ExprKind::Identifier(ident) => self.get_binding_type(&ident),

      ExprKind::Literal(lit) => self.get_node_type(&lit.id),

      ExprKind::Interpolation(parts) => {
        for part in parts {
          if let Ok(part_type) = self.get_expr_type(&part) {
            if !part_type.is_core_string() {
              self.error(AnalysisError {
                pos: part.pos,
                kind: AnalysisErrorKind::TypeMismatch {
                  expected: Type::CoreString,
                  actual: part_type.clone(),
                },
              });
            }
          }
        }

        Ok(Type::CoreString)
      }

      ExprKind::Tuple(entries) => {
        let mut entry_types = vec![];

        for entry in entries {
          let entry_type = self.get_expr_type(&entry)?;
          entry_types.push(entry_type)
        }

        Ok(Type::Tuple(entry_types))
      }

      other => todo!("support more kinds! ({:#?})", other),
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

impl Visitor for Analyzer {
  fn enter_module(&mut self, node: &ModuleNode) {
    self.push_scope();
  }

  fn leave_module(&mut self, node: &ModuleNode) {
    self.pop_scope();
  }

  fn leave_let(&mut self, node: &LetNode) {
    // println!("{:#?}", self.scopes);

    match &node.pattern.kind {
      PatternKind::Ident(ident) => self.add_binding(ident.name.clone(), ident.pos, node.value.id),
    };
  }

  fn leave_expr(&mut self, node: &ExprNode) {
    match self.get_expr_type(node) {
      Ok(typ) => self.add_node_type(node.id, typ),
      Err(err) => self.error(err),
    }
  }

  fn enter_identifier(&mut self, node: &IdentifierNode) {
    println!("ID: {}", node.name);

    println!("--> type: {:#?}", self.get_binding_type(&node))
  }

  fn enter_literal(&mut self, node: &LiteralNode) {
    let node_type = match &node.kind {
      LiteralKind::IntDecimal(_)
      | LiteralKind::IntBinary(_)
      | LiteralKind::IntHex(_)
      | LiteralKind::IntOctal(_) => Type::CoreInt,
      LiteralKind::FloatDecimal(_) => Type::CoreFloat,
      LiteralKind::Str(_) => Type::CoreString,
    };

    self.add_node_type(node.id, node_type);
  }

  fn leave_statement(&mut self, node: &StatementNode) {
    match &node.kind {
      StatementKind::Expr(expr) => match self.get_expr_type(expr) {
        Ok(typ) => self.add_node_type(node.id, typ),
        Err(err) => self.error(err),
      },
      _ => {}
    };
  }
}

#[derive(Debug)]
struct Binding {
  node_id: Uuid,
  pos: (usize, usize),
  mutable: bool,
  references: usize,
}

#[derive(Debug)]
struct Scope {
  bindings: HashMap<String, Binding>,
}

impl Scope {
  fn new() -> Self {
    Scope {
      bindings: HashMap::new(),
    }
  }

  fn add_binding(&mut self, name: String, node_id: Uuid, pos: (usize, usize), mutable: bool) {
    self.bindings.insert(
      name,
      Binding {
        node_id,
        pos,
        mutable,
        references: 0,
      },
    );
  }

  fn add_reference(&mut self, name: &String) {
    self.bindings.get_mut(name).map(|b| b.references += 1);
  }
}
