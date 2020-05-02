use crate::analysis_error::{AnalysisError, AnalysisErrorKind};
use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::visitor::Visitor;
use std::collections::HashMap;
use uuid::Uuid;

pub struct Analyzer {
  pub diagnostics: Vec<Diagnostic>,
  scopes: Vec<Scope>,
}

impl Analyzer {
  pub fn new() -> Analyzer {
    Analyzer {
      diagnostics: Vec::new(),
      scopes: Vec::new(),
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

  fn add_node_type(&mut self, node_id: Uuid, node_type: String) {
    let current_scope = self.scopes.last_mut().unwrap();

    current_scope.add_node_type(node_id, node_type);
  }

  fn add_binding(&mut self, name: String, pos: (usize, usize), node_id: Uuid) {
    let current_scope = self.scopes.last_mut().unwrap();

    current_scope.add_binding(name, node_id, pos, false);
  }

  fn add_reference(&mut self, name: &String) {
    let current_scope = self.scopes.last_mut().unwrap();

    current_scope.add_reference(name);
  }

  fn get_binding_type(&mut self, node: &IdentifierNode) -> Result<String, AnalysisError> {
    let current_scope = self.scopes.last().expect("should always have a scope");

    match current_scope.bindings.get(&node.name) {
      Some(binding) => {
        let t = current_scope
          .node_types
          .get(&binding.node_id)
          .unwrap()
          .clone();
        self.add_reference(&node.name);
        Ok(t)
      }
      None => Err(AnalysisError {
        pos: node.pos,
        kind: AnalysisErrorKind::UndefinedVariable(node.name.clone()),
      }),
    }
  }

  fn get_node_type(&self, node_id: &Uuid) -> Result<String, AnalysisError> {
    let current_scope = self.scopes.last().unwrap();

    match current_scope.node_types.get(node_id) {
      Some(node_type) => Ok(node_type.clone()),
      None => unreachable!("no type for node {}", node_id),
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
    let type_lookup = match &node.kind {
      ExprKind::Identifier(ident) => self.get_binding_type(&ident),
      ExprKind::Literal(lit) => self.get_node_type(&lit.id),
      other => todo!("support more kinds! ({:#?})", other),
    }
    .map(|t| t.to_string());

    match type_lookup {
      Ok(typ) => self.add_node_type(node.id, typ),
      Err(err) => self.error(err),
    }
  }

  fn enter_identifier(&mut self, node: &IdentifierNode) {
    println!("ID: {}", node.name);

    println!("--> type: {:#?}", self.get_binding_type(&node))
  }

  fn enter_literal(&mut self, node: &LiteralNode) {
    if let LiteralKind::IntDecimal(val) = node.kind {
      self.add_node_type(node.id, "Int".to_owned());
    }
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
  node_types: HashMap<Uuid, String>,
}

impl Scope {
  fn new() -> Self {
    Scope {
      bindings: HashMap::new(),
      node_types: HashMap::new(),
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

  fn add_node_type(&mut self, node_id: Uuid, node_type: String) {
    self.node_types.insert(node_id, node_type);
  }

  fn add_reference(&mut self, name: &String) {
    self.bindings.get_mut(name).map(|b| b.references += 1);
  }
}
