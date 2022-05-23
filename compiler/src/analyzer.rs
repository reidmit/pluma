use crate::ast::*;
use crate::binding::*;
use crate::diagnostic::*;
use crate::errors::*;
use crate::module::Module;
use crate::types::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use AnalysisErrorKind::*;

pub struct Analyzer<'compiler> {
  module_name: Option<String>,
  module_path: Option<PathBuf>,
  diagnostics: &'compiler mut Vec<Diagnostic>,
  value_scopes: Vec<HashMap<String, ValueBinding>>,
  next_type_var_id: usize,
}

impl<'compiler> Analyzer<'compiler> {
  pub fn new(diagnostics: &'compiler mut Vec<Diagnostic>) -> Self {
    Self {
      module_name: None,
      module_path: None,
      diagnostics,
      value_scopes: Vec::new(),
      next_type_var_id: 0,
    }
  }

  pub fn analyze(&mut self, module: &mut Module) {
    self.module_name = Some(module.module_name.clone());
    self.module_path = Some(module.module_path.clone());

    self.enter_scope();

    if let Some(ast) = &mut module.ast {
      let constraints = self.constrain(ast);
      let substitution = self.unify(&constraints);
      self.annotate(ast, &substitution);
    }
  }

  fn diagnostic(&mut self, span: Option<(usize, usize)>, diag: Diagnostic) {
    let mut diag = diag;

    if let Some(span) = span {
      diag = diag.with_span(span);
    }

    if let Some(module_name) = &self.module_name {
      diag = diag.with_module(module_name.clone(), self.module_path.clone().unwrap())
    }

    self.diagnostics.push(diag)
  }

  fn warning(&mut self, span: (usize, usize), kind: AnalysisErrorKind) {
    self.diagnostic(
      Some(span),
      Diagnostic::warning(AnalysisError { span, kind }),
    );
  }

  fn error(&mut self, span: (usize, usize), kind: AnalysisErrorKind) {
    self.diagnostic(Some(span), Diagnostic::error(AnalysisError { span, kind }));
  }

  fn add_value_binding(&mut self, name: String, ty_scheme: Scheme, span: (usize, usize)) {
    let current_level = self.value_scopes.last_mut().expect("no current scope");

    current_level.insert(
      name,
      ValueBinding {
        ty_scheme,
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

  fn enter_scope(&mut self) {
    self.value_scopes.push(HashMap::new());
  }

  pub fn leave_scope(&mut self) {
    if let Some(exited_level) = self.value_scopes.pop() {
      for (name, binding) in exited_level {
        if binding.ref_count == 0 && !name.starts_with("_") {
          self.warning(binding.span, UnusedBinding { name });
        }
      }
    }
  }

  fn constrain(&mut self, module: &mut ModuleNode) -> Vec<Constraint> {
    let mut constraints = Vec::new();

    // we first do a shallow pass to annotate all top-level defs and add them to the scope,
    // so that they can be referenced anywhere within the bodies of other defs
    for definition in &mut module.body {
      definition.ty = self.new_type_var();

      match &mut definition.kind {
        DefinitionKind::Expr(_) => self.add_value_binding(
          definition.name.name.clone(),
          Scheme::Forall(vec![], definition.ty.clone()),
          definition.name.span,
        ),
        _ => {
          // todo :---)
        }
      }
    }

    // then, we go through and generate constraints from the defs
    for definition in &mut module.body {
      match &mut definition.kind {
        DefinitionKind::Expr(expr) => {
          self.constrain_expr(expr, &mut constraints);

          constraints.push(eq_constraint(definition.ty.clone(), expr.ty.clone()));
        }

        _ => {
          // todo :---)
        }
      }
    }

    constraints
  }

  fn constrain_expr(&mut self, expr: &mut ExprNode, constraints: &mut Vec<Constraint>) {
    use Constraint::*;

    match &mut expr.kind {
      ExprKind::Identifier(ident) => {
        if let Some(binding) = self.get_value_binding(&ident.name) {
          return match &binding.ty_scheme {
            Scheme::Forall(_, ty) => {
              expr.ty = ty.clone();
            }

            Scheme::Var(var) => {
              // not sure about all this...
              let var = *var;
              let expr_ty = self.new_type_var();
              expr.ty = expr_ty.clone();
              constraints.push(Inst(var, expr_ty))
            }
          };
        };

        self.error(
          ident.span,
          NameNotBound {
            name: ident.name.clone(),
          },
        );

        expr.ty = Type::Unknown;
      }

      ExprKind::Literal(literal) => match &mut literal.kind {
        LiteralKind::Bool(..) => expr.ty = Type::Bool,
        LiteralKind::Str(..) => expr.ty = Type::String,
        LiteralKind::FloatDecimal(..) => expr.ty = Type::Float,
        LiteralKind::IntDecimal(..)
        | LiteralKind::IntHex(..)
        | LiteralKind::IntBinary(..)
        | LiteralKind::IntOctal(..) => expr.ty = Type::Int,
      },

      ExprKind::Regex(..) => expr.ty = Type::Regex,

      ExprKind::EmptyTuple => expr.ty = Type::Nothing,

      ExprKind::Interpolation(parts) => {
        for part in parts {
          self.constrain_expr(part, constraints);
          // each part must have type string
          constraints.push(eq_constraint(part.ty.clone(), Type::String).at(part.span));
        }

        expr.ty = Type::String;
      }

      ExprKind::Grouping(inner) => {
        let expr_ty = self.new_type_var();
        expr.ty = expr_ty.clone();

        self.constrain_expr(inner, constraints);

        constraints.push(eq_constraint(expr_ty, inner.ty.clone()));
      }

      ExprKind::Tuple(elements) => {
        expr.ty = self.new_type_var();

        let mut element_types = Vec::new();

        for element in elements {
          self.constrain_expr(element, constraints);
          element_types.push(element.ty.clone());
        }

        constraints.push(eq_constraint(expr.ty.clone(), Type::Tuple(element_types)).at(expr.span))
      }

      ExprKind::BinaryOperation { left, right, op } => {
        self.constrain_expr(left, constraints);
        self.constrain_expr(right, constraints);

        match &op.kind {
          Operator::Addition
          | Operator::SubtractionOrNegation
          | Operator::Multiplication
          | Operator::Division
          | Operator::Remainder => {
            expr.ty = Type::Int;
            constraints.push(eq_constraint(left.ty.clone(), Type::Int).at(left.span));
            constraints.push(eq_constraint(right.ty.clone(), Type::Int).at(right.span));
          }

          Operator::LogicalAnd | Operator::LogicalOr => {
            expr.ty = Type::Bool;
            constraints.push(eq_constraint(left.ty.clone(), Type::Bool).at(left.span));
            constraints.push(eq_constraint(right.ty.clone(), Type::Bool).at(right.span));
          }

          other => {
            // todo :----)
            println!("found unhandled binary op: {}", other)
          }
        }
      }

      ExprKind::Lambda(LambdaNode { params, body, .. }) => {
        expr.ty = self.new_type_var();

        let mut param_types = Vec::new();

        self.enter_scope();

        if params.is_empty() {
          param_types.push(Type::Nothing)
        } else {
          for param in params {
            param.ty = self.new_type_var();

            param_types.push(param.ty.clone());

            self.add_value_binding(
              param.ident.name.clone(),
              Scheme::Forall(vec![], param.ty.clone()),
              param.ident.span,
            )
          }
        }

        let mut return_type = Type::Nothing;

        for expr in body {
          self.constrain_expr(expr, constraints);
          return_type = expr.ty.clone();
        }

        self.leave_scope();

        // we know that this lambda must be a function that takes
        // the param types and returns the return type
        constraints.push(
          eq_constraint(
            expr.ty.clone(),
            Type::Fun(param_types, Box::new(return_type)),
          )
          .at(expr.span),
        );
      }

      ExprKind::Call(CallNode { callee, args, .. }) => {
        expr.ty = self.new_type_var();

        self.constrain_expr(callee, constraints);

        let mut arg_types = Vec::new();

        for arg in args {
          self.constrain_expr(arg, constraints);
          arg_types.push(arg.ty.clone());
        }

        // we know that the callee should be a function that takes
        // the given arg types and returns the type of this whole expr
        constraints.push(
          eq_constraint(
            callee.ty.clone(),
            Type::Fun(arg_types, expr.ty.clone().into()),
          )
          .at(expr.span),
        );
      }

      ExprKind::Let(LetNode { name, value, .. }) => {
        // visit the value (expression after the `=`), and collect constraints:
        self.constrain_expr(value, constraints);

        // add a new type scheme to the context with a new var:
        let type_scheme = self.new_type_scheme_var();
        self.add_value_binding(name.name.clone(), type_scheme.clone(), name.span);

        // not sure what this is doing...?
        constraints.push(Gen(type_scheme, value.ty.clone()));

        // let expressions always evaluate to ()
        expr.ty = Type::Nothing;
      }

      _ => {
        // todo :---)
      }
    }
  }

  fn unify(&mut self, constraints: &[Constraint]) -> Substitution {
    // split eq constraints out from others, so we can handle them in two passes
    let mut eq_constraints = Vec::new();
    let mut other_constraints = Vec::new();
    for constraint in constraints {
      if let Constraint::Eq(..) = constraint {
        eq_constraints.push(constraint.clone())
      } else {
        other_constraints.push(constraint.clone())
      }
    }

    // first pass handles eq constraints
    let subst1 = self.unify_eq_constraints(&eq_constraints);
    let other_constraints = subst1.apply_to_constraints(&other_constraints);

    // next pass handles gen/inst constraints
    let subst2 = self.unify_gen_inst_constraints(&other_constraints);

    subst1.compose(subst2)
  }

  fn unify_eq_constraints(&mut self, constraints: &[Constraint]) -> Substitution {
    if constraints.is_empty() {
      return Substitution::empty();
    }

    // first, unify the first one and get any substitutions
    let subst_first = self.unify_eq_constraint(&constraints[0]);

    // then, apply those substitutions to the remaining constraints
    let rest = subst_first.apply_to_constraints(&constraints[1..]);

    // and recursively unify the remaining (substituted) constraints
    let subst_rest = self.unify(&rest);

    // finally, return all the collected merged substitutions together
    subst_first.compose(subst_rest)
  }

  fn unify_eq_constraint(&mut self, constraint: &Constraint) -> Substitution {
    use Constraint::*;

    match constraint {
      // same types, and both are "leaf" nodes; nothing to add to the solution
      Eq(Type::Int, Type::Int, _)
      | Eq(Type::Float, Type::Float, _)
      | Eq(Type::Bool, Type::Bool, _)
      | Eq(Type::String, Type::String, _)
      | Eq(Type::Regex, Type::Regex, _)
      | Eq(Type::Nothing, Type::Nothing, _)
      | Eq(Type::Unknown, Type::Unknown, _) => Substitution::empty(),

      Eq(
        Type::Fun(param_types_1, return_type_1),
        Type::Fun(param_types_2, return_type_2),
        reason,
      ) => {
        if param_types_1.len() != param_types_2.len() {
          self.error(
            reason.span,
            ParamCountMismatch {
              expected: param_types_2.len(),
              found: param_types_1.len(),
            },
          );

          return Substitution::empty();
        }

        // add some new constraints to unify param & return types:
        let mut constraints = Vec::with_capacity(param_types_1.len() + 1);

        for i in 0..param_types_1.len() {
          constraints.push(eq_constraint(
            param_types_1[i].clone(),
            param_types_2[i].clone(),
          ))
        }

        constraints.push(eq_constraint(
          *return_type_1.clone(),
          *return_type_2.clone(),
        ));

        self.unify(&constraints)
      }

      Eq(Type::Tuple(element_types_1), Type::Tuple(element_types_2), reason) => {
        if element_types_1.len() != element_types_2.len() {
          self.error(
            reason.span,
            TupleSizeMismatch {
              expected: element_types_2.len(),
              found: element_types_1.len(),
            },
          );

          return Substitution::empty();
        }

        // add some new constraints to unify element types:
        let mut constraints = Vec::with_capacity(element_types_2.len() + 1);

        for i in 0..element_types_1.len() {
          constraints.push(eq_constraint(
            element_types_1[i].clone(),
            element_types_2[i].clone(),
          ))
        }

        self.unify(&constraints)
      }

      Eq(Type::Var(n), t, reason) | Eq(t, Type::Var(n), reason) => match t {
        Type::Var(n2) if n == n2 => Substitution::empty(),
        Type::Var(_) => Substitution::with_entry(*n, t.clone()),
        other => {
          if other.contains_var(*n) {
            self.error(reason.span, RecursiveUnification { ty: other.clone() });
            return Substitution::empty();
          }

          Substitution::with_entry(*n, t.clone())
        }
      },

      Eq(a, b, reason) => {
        self.error(
          reason.span,
          TypeMismatch {
            expected: b.clone(),
            found: a.clone(),
          },
        );

        Substitution::empty()
      }

      _ => {
        unreachable!("should only have eq constraints in here")
      }
    }
  }

  fn unify_gen_inst_constraints(&mut self, constraints: &[Constraint]) -> Substitution {
    if constraints.is_empty() {
      return Substitution::empty();
    }

    match &constraints[0] {
      Constraint::Gen(scheme, ty) => {
        let mut inst_constraints_for_gen = Vec::new();
        let mut other_constraints = Vec::new();
        for constraint in &constraints[1..] {
          match (constraint, scheme) {
            (Constraint::Inst(var1, ..), Scheme::Var(var2, ..)) if *var1 == *var2 => {
              inst_constraints_for_gen.push(constraint.clone())
            }
            _ => other_constraints.push(constraint.clone()),
          }
        }

        let new_eq_constraints = self.instantiate_constraints(&inst_constraints_for_gen, &ty);
        let subst = self.unify_eq_constraints(&new_eq_constraints);
        let other_constraints = subst.apply_to_constraints(&other_constraints);
        let subst2 = self.unify_gen_inst_constraints(&other_constraints);

        subst.compose(subst2)
      }

      _ => unreachable!("should have a Gen first"),
    }
  }

  fn fill_in_placeholder(&mut self, ty: &mut Type, subst: &Substitution) {
    if let Type::Var(n) = ty {
      if let Some(actual_type) = subst.solutions.get(&n) {
        *ty = actual_type.clone();
      }
    }
  }

  fn annotate(&mut self, module: &mut ModuleNode, subst: &Substitution) {
    for definition in &mut module.body {
      self.annotate_definition(definition, subst)
    }
  }

  fn annotate_definition(&mut self, definition: &mut DefinitionNode, subst: &Substitution) {
    self.fill_in_placeholder(&mut definition.ty, subst);

    println!("{} :: {}", definition.name.name, definition.ty);

    match &mut definition.kind {
      DefinitionKind::Expr(expr) => self.annotate_expr(expr, subst),
      _ => { /* todo */ }
    }
  }

  fn annotate_expr(&mut self, expr: &mut ExprNode, subst: &Substitution) {
    self.fill_in_placeholder(&mut expr.ty, subst);

    match &mut expr.kind {
      ExprKind::Let(LetNode { value, .. }) => {
        self.annotate_expr(value, subst);
      }

      ExprKind::Lambda(LambdaNode { params, body, .. }) => {
        for param in params {
          self.fill_in_placeholder(&mut param.ty, subst);
        }

        for expr in body {
          self.annotate_expr(expr, subst);
        }
      }

      ExprKind::Call(CallNode { callee, args, .. }) => {
        self.annotate_expr(callee, subst);

        for arg in args {
          self.annotate_expr(arg, subst);
        }
      }

      ExprKind::Tuple(elements) => {
        for element in elements {
          self.annotate_expr(element, subst);
        }
      }

      _ => {}
    }
  }

  fn instantiate_constraints(&mut self, constraints: &[Constraint], ty: &Type) -> Vec<Constraint> {
    let mut new_constraints = Vec::new();

    let scheme = self.generalize_type(ty);

    for constraint in constraints {
      if let Constraint::Inst(_, ty) = constraint {
        let instantiated_ty = self.instantiate_scheme(&scheme);
        new_constraints.push(eq_constraint(ty.clone(), instantiated_ty));
      } else {
        unreachable!("should only have inst constraints here");
      }
    }

    new_constraints
  }

  fn instantiate_scheme(&mut self, scheme: &Scheme) -> Type {
    match scheme {
      Scheme::Var(_) => unreachable!("shouldn't be instantiating a scheme var"),
      Scheme::Forall(vars, ty) => {
        // generate a new fresh type var for each of the forall vars
        let mut subst = Substitution::empty();
        for var in vars {
          subst.solutions.insert(*var, self.new_type_var());
        }

        // and then apply that substitution in ty
        subst.apply_to_type(ty)
      }
    }
  }

  fn generalize_type(&self, ty: &Type) -> Scheme {
    let mut vars = HashSet::new();

    // add all free vars in ty
    for var in ty.free_vars() {
      vars.insert(var);
    }

    // remove all free vars in context
    for (_, binding) in self.value_scopes.last().unwrap() {
      // todo: all scope levels?
      for var in binding.ty_scheme.free_vars() {
        vars.remove(&var);
      }
    }

    Scheme::Forall(Vec::from_iter(vars), ty.clone())
  }

  fn new_type_scheme_var(&mut self) -> Scheme {
    let type_var = Scheme::Var(self.next_type_var_id);
    self.next_type_var_id += 1;
    type_var
  }

  fn new_type_var(&mut self) -> Type {
    let type_var = Type::Var(self.next_type_var_id);
    self.next_type_var_id += 1;
    type_var
  }
}
