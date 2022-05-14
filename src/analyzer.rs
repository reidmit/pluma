use crate::ast::*;
use crate::binding::*;
use crate::diagnostic::*;
use crate::errors::*;
use crate::module::Module;
use crate::value_type::*;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Analyzer<'compiler> {
  module_name: Option<String>,
  module_path: Option<PathBuf>,
  diagnostics: &'compiler mut Vec<Diagnostic>,
  scope_levels: Vec<HashMap<String, Binding>>,
}

impl<'compiler> Analyzer<'compiler> {
  pub fn new(diagnostics: &'compiler mut Vec<Diagnostic>) -> Self {
    Self {
      module_name: None,
      module_path: None,
      diagnostics,
      scope_levels: Vec::new(),
    }
  }

  pub fn analyze(&mut self, module: &mut Module) {
    self.module_name = Some(module.module_name.clone());
    self.module_path = Some(module.module_path.clone());

    if let Some(ast) = &mut module.ast {
      self.enter_scope();

      for definition in &mut ast.body {
        self.analyze_definition(definition)
      }

      // we intentionally don't call leave_scope() here, to avoid warnings
      // about top-level bindings being unused
      self.scope_levels.pop();
    }
  }

  fn warning(&mut self, pos: (usize, usize), kind: AnalysisErrorKind) {
    let mut diagnostic = Diagnostic::warning(AnalysisError { pos, kind }).with_pos(pos);

    if let Some(module_name) = &self.module_name {
      diagnostic = diagnostic.with_module(module_name.clone(), self.module_path.clone().unwrap())
    }

    self.diagnostics.push(diagnostic)
  }

  fn error(&mut self, pos: (usize, usize), kind: AnalysisErrorKind) {
    let mut diagnostic = Diagnostic::error(AnalysisError { pos, kind }).with_pos(pos);

    if let Some(module_name) = &self.module_name {
      diagnostic = diagnostic.with_module(module_name.clone(), self.module_path.clone().unwrap())
    }

    self.diagnostics.push(diagnostic)
  }

  fn enter_scope(&mut self) {
    self.scope_levels.push(HashMap::new());
  }

  pub fn leave_scope(&mut self) {
    if let Some(exited_level) = self.scope_levels.pop() {
      for (name, binding) in exited_level {
        if binding.ref_count == 0 {
          self.warning(binding.pos, AnalysisErrorKind::UnusedBinding { name });
        }
      }
    }
  }

  pub fn add_binding(
    &mut self,
    name: String,
    typ: ValueType,
    pos: (usize, usize),
    kind: BindingKind,
  ) {
    let current_level = self.scope_levels.last_mut().expect("no current scope");

    current_level.insert(
      name,
      Binding {
        typ,
        ref_count: 0,
        pos,
        kind,
      },
    );
  }

  pub fn get_binding(&mut self, name: &String) -> Option<&Binding> {
    for level in self.scope_levels.iter_mut().rev() {
      if let Some(binding) = level.get_mut(name) {
        binding.ref_count += 1;

        return Some(binding);
      }
    }

    None
  }

  fn analyze_definition(&mut self, definition: &mut DefinitionNode) {
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

    self.add_binding(name, resolved_type, definition.name.pos, BindingKind::Def)
  }

  fn analyze_expr(&mut self, expr: &mut ExprNode) -> ValueType {
    match &mut expr.kind {
      ExprKind::Identifier(ident) => self.analyze_identifier(ident),
      ExprKind::Literal(literal) => self.analyze_literal(literal),
      ExprKind::Tuple(entries) => self.analyze_tuple_entries(entries),
      ExprKind::EmptyTuple => ValueType::Nothing,
      ExprKind::Lambda(lambda) => self.analyze_lambda(lambda),
      ExprKind::Let(let_node) => self.analyze_let(let_node),
      ExprKind::Interpolation(parts) => self.analyze_interpolation(parts),
      ExprKind::Grouping(inner) => self.analyze_expr(inner),
      // TODO! more here!
      _ => ValueType::Unknown,
    }
  }

  fn analyze_lambda(&mut self, lambda: &mut LambdaNode) -> ValueType {
    let mut param_types = Vec::new();
    let mut return_type = ValueType::Unknown;

    self.enter_scope();

    for param in &lambda.params {
      let name = param.name.clone();

      self.add_binding(name, ValueType::Unknown, param.pos, BindingKind::Param);

      param_types.push(ValueType::Unknown);
    }

    for expr in &mut lambda.body {
      return_type = self.analyze_expr(expr);
    }

    self.leave_scope();

    ValueType::Func(param_types, Box::new(return_type))
  }

  fn analyze_identifier(&mut self, ident: &mut IdentifierNode) -> ValueType {
    if let Some(binding) = self.get_binding(&ident.name) {
      binding.typ.clone()
    } else {
      self.error(
        ident.pos,
        AnalysisErrorKind::NameNotBound {
          name: ident.name.clone(),
        },
      );

      ValueType::Unknown
    }
  }

  fn analyze_let(&mut self, let_node: &mut LetNode) -> ValueType {
    let name = let_node.name.name.clone();
    let value_type = self.analyze_expr(&mut let_node.value);

    self.add_binding(
      name,
      value_type.clone(),
      let_node.name.pos,
      BindingKind::Let,
    );

    value_type
  }

  fn analyze_interpolation(&mut self, parts: &mut Vec<ExprNode>) -> ValueType {
    for part in parts {
      match self.analyze_expr(part) {
        ValueType::String => {}
        other_type => self.error(
          part.pos,
          AnalysisErrorKind::MismatchedTypes {
            expected: ValueType::String,
            actual: other_type,
          },
        ),
      }
    }

    ValueType::String
  }

  fn analyze_tuple_entries(&mut self, entries: &mut Vec<TupleEntry>) -> ValueType {
    let mut entry_types = Vec::new();

    for TupleEntry(maybe_label, value) in entries {
      let entry_label = match maybe_label {
        Some(ident) => Some(ident.name.clone()),
        None => None,
      };

      let entry_type = self.analyze_expr(value);

      entry_types.push((entry_label, entry_type));
    }

    ValueType::Tuple(entry_types)
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
}
