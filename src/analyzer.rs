use crate::ast::*;
use crate::binding::*;
use crate::diagnostic::*;
use crate::errors::*;
use crate::expr_type::*;
use crate::intrinsics::*;
use crate::module::Module;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use AnalysisErrorKind::*;

type ConstraintSet = HashSet<(ExprType, ExprType)>;
type SolutionMap = HashMap<usize, ExprType>;

pub struct Analyzer<'compiler> {
  module_name: Option<String>,
  module_path: Option<PathBuf>,
  diagnostics: &'compiler mut Vec<Diagnostic>,
  type_scope: HashMap<String, TypeBinding>,
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
      // initialize top-level scopes with intrinsics:
      type_scope: get_intrinsic_types(),
      value_scopes: vec![get_intrinsic_values()],
      next_placeholder_id: 0,
    }
  }

  pub fn analyze(&mut self, module: &mut Module) {
    self.module_name = Some(module.module_name.clone());
    self.module_path = Some(module.module_path.clone());

    if let Some(ast) = &mut module.ast {
      // phase 1: annotate w/placeholders
      self.annotate(ast);

      // phase 2: generate constraints
      let mut constraints = self.generate_constraints(ast);

      // phase 3: solve constraints
      let solutions = self.solve_constraints(&mut constraints);

      // phase 4: annotate w/inferred types
      // TODO :---)
      println!("{:#?}", solutions);
    }
  }
}

// Helper methods
impl<'compiler> Analyzer<'compiler> {
  fn diagnostic(&mut self, loc: (usize, usize), diag: Diagnostic) {
    let mut diag = diag.with_pos(loc);

    if let Some(module_name) = &self.module_name {
      diag = diag.with_module(module_name.clone(), self.module_path.clone().unwrap())
    }

    self.diagnostics.push(diag)
  }

  fn warning(&mut self, loc: (usize, usize), kind: AnalysisErrorKind) {
    self.diagnostic(loc, Diagnostic::warning(AnalysisError { loc, kind }));
  }

  fn error(&mut self, loc: (usize, usize), kind: AnalysisErrorKind) {
    self.diagnostic(loc, Diagnostic::error(AnalysisError { loc, kind }));
  }

  fn enter_scope(&mut self) {
    self.value_scopes.push(HashMap::new());
  }

  pub fn leave_scope(&mut self) {
    if let Some(exited_level) = self.value_scopes.pop() {
      for (name, binding) in exited_level {
        if binding.ref_count == 0 {
          self.warning(binding.loc, UnusedBinding { name });
        }
      }
    }
  }

  fn new_placeholder_type(&mut self) -> ExprType {
    let placeholder_id = self.next_placeholder_id;
    self.next_placeholder_id += 1;
    ExprType::Placeholder(placeholder_id)
  }

  fn add_value_binding(&mut self, name: String, typ: ExprType, loc: (usize, usize)) {
    let current_level = self.value_scopes.last_mut().expect("no current scope");

    current_level.insert(
      name,
      ValueBinding {
        typ,
        ref_count: 0,
        loc,
      },
    );
  }

  pub fn add_type_binding(&mut self, name: String, typ: ExprType, loc: (usize, usize)) {
    self.type_scope.insert(name, TypeBinding { typ, loc });
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

  pub fn get_type_binding(&self, name: &String) -> Option<&TypeBinding> {
    if let Some(binding) = self.type_scope.get(name) {
      return Some(binding);
    }

    None
  }

  pub fn get_field_type(&self, typ: &ExprType, field_name: &String) -> Option<ExprType> {
    match typ {
      ExprType::Tuple(entries) => {
        let mut index = 0;

        for entry_type in entries {
          if *field_name == format!("{}", index) {
            return Some(entry_type.clone());
          }

          index = index + 1;
        }

        None
      }

      ExprType::Record(entries) => {
        for (label, entry_type) in entries {
          if *field_name == *label {
            return Some(entry_type.clone());
          }
        }

        None
      }

      ExprType::Named(name) => {
        if let Some(binding) = self.get_type_binding(name) {
          return self.get_field_type(&binding.typ, field_name);
        }

        None
      }

      _ => None,
    }
  }
}

// Annotation methods
impl<'compiler> Analyzer<'compiler> {
  fn annotate(&mut self, module: &mut ModuleNode) {
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
        definition.name.loc,
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
            ident.loc,
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
          self.add_value_binding(ident.name.clone(), param_type.clone(), ident.loc);
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
        self.add_value_binding(name.name.clone(), binding_type, name.loc);

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
    let mut constraints = HashSet::new();

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
        constraints.insert((definition.inferred_type.clone(), expr.inferred_type.clone()));
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
        constraints.insert((inferred_type, ExprType::Regex));
      }

      ExprKind::Grouping(inner) => {
        self.constraints_from_expr(inner, constraints);
        constraints.insert((inferred_type, inner.inferred_type.clone()));
      }

      ExprKind::BinaryOperation { left, right, op } => {
        self.constraints_from_expr(left, constraints);
        self.constraints_from_expr(right, constraints);

        match op.kind {
          Operator::Addition => {
            // todo: floats?
            constraints.insert((left.inferred_type.clone(), ExprType::Int));
            constraints.insert((right.inferred_type.clone(), ExprType::Int));
            constraints.insert((inferred_type.clone(), ExprType::Int));
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
        constraints.insert((
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
        constraints.insert((
          callee.inferred_type.clone(),
          ExprType::Func(arg_types, Box::new(inferred_type)),
        ));
      }

      ExprKind::Let(LetNode { value, .. }) => {
        self.constraints_from_expr(value, constraints);

        // let expressions always evaluate to ()
        constraints.insert((inferred_type, ExprType::Nothing));
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
        constraints.insert((typ, ExprType::String));
      }
      LiteralKind::FloatDecimal(..) => {
        constraints.insert((typ, ExprType::Float));
      }
      LiteralKind::IntDecimal(..)
      | LiteralKind::IntHex(..)
      | LiteralKind::IntBinary(..)
      | LiteralKind::IntOctal(..) => {
        constraints.insert((typ, ExprType::Int));
      }
    }
  }
}

// Constraint-solving methods
impl<'compiler> Analyzer<'compiler> {
  fn solve_constraints(&mut self, constraints: &mut ConstraintSet) -> SolutionMap {
    let mut solutions = HashMap::new();

    while !constraints.is_empty() {
      let mut remaining_constraints = HashSet::new();

      for (t1, t2) in constraints.drain() {
        self.unify(&mut remaining_constraints, &mut solutions, &t1, &t2)
      }

      *constraints = remaining_constraints;
    }

    solutions
  }

  fn unify(
    &mut self,
    remaining_constraints: &mut ConstraintSet,
    solutions: &mut SolutionMap,
    t1: &ExprType,
    t2: &ExprType,
  ) {
    match (t1, t2) {
      (t1, t2) if !t1.has_any_placeholder() && !t2.has_any_placeholder() => {
        // both are "leaf" nodes, so nothing to do
      }

      (ExprType::Placeholder(n), t) if !t.has_any_placeholder() => {
        // if a is a placeholder and b is a "leaf", add to the solution
        solutions.insert(*n, t.clone());
      }

      (t, ExprType::Placeholder(n)) if !t.has_any_placeholder() => {
        // similarly, if b is a placeholder and a is a "leaf", add to the solution
        solutions.insert(*n, t.clone());
      }

      (
        ExprType::Func(param_types_t1, return_type_t1),
        ExprType::Func(param_types_t2, return_type_t2),
      ) => {
        for i in 0..param_types_t1.len() {
          self.unify(
            remaining_constraints,
            solutions,
            &param_types_t1[i],
            &param_types_t2[i],
          )
        }

        self.unify(
          remaining_constraints,
          solutions,
          &return_type_t1,
          &return_type_t2,
        )
      }

      (type_a, type_b) => {
        // otherwise, perform any substitutions we can and include
        // in the remaining constraints
        let substituted = (
          type_a.replace_placeholders(&solutions),
          type_b.replace_placeholders(&solutions),
        );

        remaining_constraints.insert(substituted);
      }
    }
  }
}
