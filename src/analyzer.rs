use crate::ast::*;
use crate::binding::*;
use crate::diagnostic::*;
use crate::errors::*;
use crate::expr_type::*;
use crate::intrinsics::*;
use crate::module::Module;
use crate::solution_map::*;
use crate::*;
use std::collections::HashMap;
use std::path::PathBuf;
use AnalysisErrorKind::*;

pub struct Analyzer<'compiler> {
  module_name: Option<String>,
  module_path: Option<PathBuf>,
  diagnostics: &'compiler mut Vec<Diagnostic>,
  value_scopes: Vec<HashMap<String, ValueBinding>>,
  next_placeholder_id: usize,
}

// Public interface
impl<'compiler> Analyzer<'compiler> {
  pub fn new(diagnostics: &'compiler mut Vec<Diagnostic>) -> Self {
    Self {
      module_name: None,
      module_path: None,
      diagnostics,
      value_scopes: vec![get_intrinsic_values()],
      next_placeholder_id: 0,
    }
  }

  pub fn analyze(&mut self, module: &mut Module) {
    self.module_name = Some(module.module_name.clone());
    self.module_path = Some(module.module_path.clone());

    if let Some(ast) = &mut module.ast {
      self.annotate_with_placeholders(ast);

      let constraints = self.generate_constraints(ast);
      let solutions = self.unify_constraints(&constraints);

      self.decorate_with_inferred_types(ast, &solutions);
    }
  }
}

// Helper methods
impl<'compiler> Analyzer<'compiler> {
  fn diagnostic(&mut self, span: (usize, usize), diag: Diagnostic) {
    let mut diag = diag.with_pos(span);

    if let Some(module_name) = &self.module_name {
      diag = diag.with_module(module_name.clone(), self.module_path.clone().unwrap())
    }

    self.diagnostics.push(diag)
  }

  fn warning(&mut self, span: (usize, usize), kind: AnalysisErrorKind) {
    self.diagnostic(span, Diagnostic::warning(AnalysisError { span, kind }));
  }

  fn error(&mut self, span: (usize, usize), kind: AnalysisErrorKind) {
    self.diagnostic(span, Diagnostic::error(AnalysisError { span, kind }));
  }

  fn enter_scope(&mut self) {
    self.value_scopes.push(HashMap::new());
  }

  pub fn leave_scope(&mut self) {
    if let Some(exited_level) = self.value_scopes.pop() {
      for (name, binding) in exited_level {
        if binding.ref_count == 0 {
          self.warning(binding.span, UnusedBinding { name });
        }
      }
    }
  }

  fn new_placeholder_type(&mut self) -> ExprType {
    let placeholder_id = self.next_placeholder_id;
    self.next_placeholder_id += 1;
    ExprType::Placeholder(placeholder_id)
  }

  fn add_value_binding(&mut self, name: String, typ: ExprType, span: (usize, usize)) {
    let current_level = self.value_scopes.last_mut().expect("no current scope");

    current_level.insert(
      name,
      ValueBinding {
        typ,
        ref_count: 0,
        span,
      },
    );
  }

  pub fn get_value_binding(&mut self, name: &String) -> Option<&ValueBinding> {
    for level in self.value_scopes.iter_mut().rev() {
      if let Some(binding) = level.get_mut(name) {
        binding.ref_count += 1;

        return Some(binding);
      }
    }

    None
  }
}

// Annotation methods
impl<'compiler> Analyzer<'compiler> {
  fn annotate_with_placeholders(&mut self, module: &mut ModuleNode) {
    // we first do a shallow pass to annotate all top-level defs,
    // so that they can be referenced anywhere within the bodies
    // of other defs
    for definition in &mut module.body {
      self.annotate_definition(definition)
    }

    // and then we do a deeper pass over the def bodies
    for definition in &mut module.body {
      self.annotate_definition_body(definition)
    }
  }

  fn annotate_definition(&mut self, definition: &mut DefinitionNode) {
    definition.inferred_type = self.new_placeholder_type();

    match &mut definition.kind {
      DefinitionKind::Expr(_) => self.add_value_binding(
        definition.name.name.clone(),
        definition.inferred_type.clone(),
        definition.name.span,
      ),
      _ => {
        // todo :---)
      }
    }
  }

  fn annotate_definition_body(&mut self, definition: &mut DefinitionNode) {
    match &mut definition.kind {
      DefinitionKind::Expr(expr) => self.annotate_expr(expr),
      _ => {
        // todo :---)
      }
    }
  }

  fn annotate_expr(&mut self, expr: &mut ExprNode) {
    match &mut expr.kind {
      ExprKind::Literal(_) | ExprKind::Regex(_) | ExprKind::EmptyTuple => {
        // these are all "leaf" nodes (can't contain inner expressions), so
        // just give them each a new placeholder type
        expr.inferred_type = self.new_placeholder_type();
      }

      ExprKind::Grouping(inner) => {
        self.annotate_expr(inner);

        expr.inferred_type = self.new_placeholder_type();
      }

      ExprKind::Identifier(ident) => {
        if let Some(binding) = self.get_value_binding(&ident.name) {
          expr.inferred_type = binding.typ.clone();
        } else {
          self.error(
            ident.span,
            NameNotBound {
              name: ident.name.clone(),
            },
          )
        }
      }

      ExprKind::BinaryOperation { left, right, .. } => {
        self.annotate_expr(left);
        self.annotate_expr(right);

        expr.inferred_type = self.new_placeholder_type();
      }

      ExprKind::Lambda(LambdaNode { params, body, .. }) => {
        self.enter_scope();

        for LambdaParamNode {
          ident,
          inferred_type,
          ..
        } in params
        {
          let param_type = self.new_placeholder_type();
          self.add_value_binding(ident.name.clone(), param_type.clone(), ident.span);
          *inferred_type = param_type;
        }

        for expr in body {
          self.annotate_expr(expr);
        }

        self.leave_scope();

        expr.inferred_type = self.new_placeholder_type();
      }

      ExprKind::Let(LetNode { name, value, .. }) => {
        let binding_type = self.new_placeholder_type();
        self.add_value_binding(name.name.clone(), binding_type, name.span);

        self.annotate_expr(value);

        expr.inferred_type = self.new_placeholder_type();
      }

      ExprKind::Call(CallNode { callee, args, .. }) => {
        self.annotate_expr(callee);

        for arg in args {
          self.annotate_expr(arg);
        }

        expr.inferred_type = self.new_placeholder_type();
      }

      // TODO! more here!
      other => {
        println!("other kind of expr: {:#?}", other)
      }
    };
  }
}

// Constraint-generation methods
impl<'compiler> Analyzer<'compiler> {
  fn generate_constraints(&mut self, module: &mut ModuleNode) -> ConstraintSet {
    let mut constraints = Vec::new();

    for definition in &mut module.body {
      self.constraints_from_definition(definition, &mut constraints)
    }

    constraints
  }

  fn constraints_from_definition(
    &mut self,
    definition: &mut DefinitionNode,
    constraints: &mut ConstraintSet,
  ) {
    match &mut definition.kind {
      DefinitionKind::Expr(expr) => {
        self.constraints_from_expr(expr, constraints);
        constraints.push((definition.inferred_type.clone(), expr.inferred_type.clone()));
      }
      _ => {
        // todo :---)
      }
    }
  }

  fn constraints_from_expr(&mut self, expr: &mut ExprNode, constraints: &mut ConstraintSet) {
    let inferred_type = expr.inferred_type.clone();

    match &mut expr.kind {
      ExprKind::Identifier(..) => { /* no constraints to add */ }

      ExprKind::Literal(literal) => {
        self.constraints_from_literal(inferred_type, literal, constraints)
      }

      ExprKind::Regex(..) => {
        constraints.push((inferred_type, ExprType::Regex));
      }

      ExprKind::Grouping(inner) => {
        self.constraints_from_expr(inner, constraints);
        constraints.push((inferred_type, inner.inferred_type.clone()));
      }

      ExprKind::BinaryOperation { left, right, op } => {
        self.constraints_from_expr(left, constraints);
        self.constraints_from_expr(right, constraints);

        match op.kind {
          Operator::Addition => {
            // todo: floats?
            constraints.push((left.inferred_type.clone(), ExprType::Int));
            constraints.push((right.inferred_type.clone(), ExprType::Int));
            constraints.push((inferred_type.clone(), ExprType::Int));
          }
          _ => {
            // todo :----)
          }
        }
      }

      ExprKind::Lambda(LambdaNode { params, body, .. }) => {
        let param_types = params.iter().map(|p| p.inferred_type.clone()).collect();

        let mut return_type = ExprType::Nothing;

        for expr in body {
          self.constraints_from_expr(expr, constraints);
          return_type = expr.inferred_type.clone();
        }

        // we know that this lambda must be a function that takes
        // the param types and returns the return type
        constraints.push((
          inferred_type,
          ExprType::Func(param_types, Box::new(return_type)),
        ));
      }

      ExprKind::Call(CallNode { callee, args, .. }) => {
        let arg_types = args.iter().map(|a| a.inferred_type.clone()).collect();

        self.constraints_from_expr(callee, constraints);

        for arg in args {
          self.constraints_from_expr(arg, constraints);
        }

        // we know that the callee should be a function that takes
        // the given arg types and returns the type of this whole expr
        constraints.push((
          callee.inferred_type.clone(),
          ExprType::Func(arg_types, Box::new(inferred_type)),
        ));
      }

      ExprKind::Let(LetNode { value, .. }) => {
        self.constraints_from_expr(value, constraints);

        // let expressions always evaluate to ()
        constraints.push((inferred_type, ExprType::Nothing));
      }

      _ => {
        // todo :---)
      }
    }
  }

  fn constraints_from_literal(
    &mut self,
    typ: ExprType,
    literal: &mut LiteralNode,
    constraints: &mut ConstraintSet,
  ) {
    match &mut literal.kind {
      LiteralKind::Str(..) => {
        constraints.push((typ, ExprType::String));
      }

      LiteralKind::FloatDecimal(..) => {
        constraints.push((typ, ExprType::Float));
      }

      LiteralKind::IntDecimal(..)
      | LiteralKind::IntHex(..)
      | LiteralKind::IntBinary(..)
      | LiteralKind::IntOctal(..) => {
        constraints.push((typ, ExprType::Int));
      }
    }
  }
}

// Constraint-solving methods
impl<'compiler> Analyzer<'compiler> {
  fn unify_constraints(&self, constraints: &ConstraintSet) -> SolutionMap {
    if constraints.is_empty() {
      return SolutionMap::empty();
    }

    let solutions_for_head = self.unify_constraint(&constraints[0]);
    let new_tail_constraints = solutions_for_head.apply_to_constraints(&constraints[1..]);
    let solutions_for_tail = self.unify_constraints(&new_tail_constraints);
    solutions_for_head.compose(solutions_for_tail)
  }

  fn unify_constraint(&self, constraint: &Constraint) -> SolutionMap {
    match constraint {
      // both are "leaf" nodes; nothing to add to the solution
      (t1, t2) if !t1.has_any_placeholders() && !t2.has_any_placeholders() => SolutionMap::empty(),

      (
        ExprType::Func(param_types_1, return_type_1),
        ExprType::Func(param_types_2, return_type_2),
      ) => {
        // add some new constraints to unify param & return types:
        let mut constraints = Vec::with_capacity(param_types_1.len() + 1);
        for i in 0..param_types_1.len() {
          constraints.push((param_types_1[i].clone(), param_types_2[i].clone()))
        }
        constraints.push((*return_type_1.clone(), *return_type_2.clone()));
        self.unify_constraints(&constraints)
      }

      (ExprType::Placeholder(n), t) | (t, ExprType::Placeholder(n)) => match t {
        ExprType::Placeholder(n2) if n == n2 => SolutionMap::empty(),
        ExprType::Placeholder(n) => SolutionMap::with_entry(*n, t.clone()),
        other => {
          if other.contains_placeholder(n) {
            todo!("circular reference! can't unify!")
          }

          SolutionMap::with_entry(*n, t.clone())
        }
      },

      other => todo!("unexpected constraint: {:?}", other),
    }
  }
}

// Decoration methods
impl<'compiler> Analyzer<'compiler> {
  fn decorate_with_inferred_types(&mut self, module: &mut ModuleNode, solutions: &SolutionMap) {
    for definition in &mut module.body {
      self.decorate_definition(definition, solutions)
    }
  }

  fn decorate_definition(&mut self, definition: &mut DefinitionNode, solutions: &SolutionMap) {
    if let ExprType::Placeholder(n) = definition.inferred_type {
      if let Some(actual_type) = solutions.solutions.get(&n) {
        definition.inferred_type = actual_type.clone();
      }
    }

    match &mut definition.kind {
      DefinitionKind::Expr(expr) => self.decorate_expr(expr, solutions),
      _ => { /* todo */ }
    }
  }

  fn decorate_expr(&mut self, expr: &mut ExprNode, solutions: &SolutionMap) {
    if let ExprType::Placeholder(n) = expr.inferred_type {
      if let Some(actual_type) = solutions.solutions.get(&n) {
        expr.inferred_type = actual_type.clone();
      }
    }
  }
}
