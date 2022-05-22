use crate::ast::*;
use crate::binding::*;
use crate::constraint::*;
use crate::diagnostic::*;
use crate::errors::*;
use crate::expr_type::*;
use crate::intrinsics::*;
use crate::module::Module;
use crate::solution_map::*;
use crate::typing::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use AnalysisErrorKind::*;
use Constraint::*;

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
    let initial_ctx = HashMap::new();

    if let Some(ast) = &mut module.ast {
      for definition in &mut ast.body {
        if let DefinitionKind::Expr(expr) = &mut definition.kind {
          let (substitution, ty) = self.infer_expr(&initial_ctx, expr);
          let inferred_type = self.substitute_in_type(&substitution, &ty);
          println!("{} :: {}", definition.name.name, inferred_type)
        }
      }
    }
  }

  fn infer_expr(&mut self, ctx: &TypeContext, expr: &ExprNode) -> (TypeSubstitution, Type) {
    match &expr.kind {
      ExprKind::Literal(literal) => self.infer_literal(&literal),

      ExprKind::Grouping(inner) => self.infer_expr(ctx, inner),

      ExprKind::Identifier(ident) => match ctx.get(&ident.name) {
        Some(scheme) => {
          let ty = self.instantiate(scheme);
          (HashMap::new(), ty)
        }

        None => panic!("unbound variable {}", ident.name),
      },

      ExprKind::Call(CallNode { callee, args, .. }) => {
        let return_type = self.new_type_var();

        let (s1, callee_type) = self.infer_expr(&ctx, &callee);

        // TODO: support multiple args
        let arg = &args[0];
        let arg_ctx = self.substitute_in_context(&s1, &ctx);
        let (s2, arg_type) = self.infer_expr(&arg_ctx, &arg);

        let inferred_callee_type = self.substitute_in_type(&s2, &callee_type);
        let expected_callee_type = Type::Fun(vec![arg_type], return_type.clone().into());

        let s3 = self.unify(&inferred_callee_type, &expected_callee_type);

        let composed_subst = self.compose_substitutions(&s3, &s2);
        let composed_subst = self.compose_substitutions(&composed_subst, &s1);

        (composed_subst, self.substitute_in_type(&s3, &return_type))
      }

      ExprKind::Lambda(LambdaNode { params, body, .. }) => {
        let mut lambda_ctx = ctx.clone();
        let mut param_types = Vec::new();

        for param in params {
          let param_type = self.new_type_var();
          param_types.push(param_type.clone());

          // within this lambda scope, extend ctx to include params
          let param_type_scheme = TypeScheme::Mono(param_type.clone());
          lambda_ctx.insert(param.ident.name.clone(), param_type_scheme);
        }

        // TODO: support multiple body exprs
        let body = body.get(0).unwrap();

        let (s1, body_type) = self.infer_expr(&lambda_ctx, &body);

        (s1, Type::Fun(param_types, body_type.into()))
      }

      other => todo!("other expr kind: {:#?}", other),
    }
  }

  fn infer_literal(&mut self, literal: &LiteralNode) -> (TypeSubstitution, Type) {
    match literal.kind {
      LiteralKind::IntDecimal(..) => (HashMap::new(), Type::Int),
      LiteralKind::Str(..) => (HashMap::new(), Type::String),
      _ => todo!("more literal kinds!"),
    }
  }

  fn instantiate(&mut self, scheme: &TypeScheme) -> Type {
    match scheme {
      TypeScheme::Mono(ty) => ty.clone(),
      TypeScheme::Poly(forall_vars, ty) => {
        println!("INSTANTIATING!!!");

        // create a substitution that replaces all forall vars with new vars
        let mut subst = HashMap::new();
        for var in forall_vars {
          subst.insert(*var, self.new_type_var());
        }

        // ...and apply it
        self.substitute_in_type(&subst, ty)
      }
    }
  }

  fn unify(&mut self, t1: &Type, t2: &Type) -> TypeSubstitution {
    match (t1, t2) {
      (Type::Int, Type::Int) => HashMap::new(),
      (Type::String, Type::String) => HashMap::new(),
      (Type::Fun(param_types_1, return_type_1), Type::Fun(param_types_2, return_type_2)) => {
        let mut s1 = HashMap::new();

        // TODO: assert same length
        for i in 0..param_types_1.len() {
          let new_subst = self.unify(&param_types_1[i], &param_types_2[i]);
          s1 = self.compose_substitutions(&s1, &new_subst);
        }

        let return_type_1 = self.substitute_in_type(&s1, return_type_1);
        let return_type_2 = self.substitute_in_type(&s1, return_type_2);
        let s2 = self.unify(&return_type_1, &return_type_2);

        self.compose_substitutions(&s2, &s1)
      }

      (Type::Var(var), ty) | (ty, Type::Var(var)) => match ty {
        Type::Var(var2) if var == var2 => HashMap::new(),
        _ => {
          // todo: occurs check here
          let mut subst = HashMap::new();
          subst.insert(*var, ty.clone());
          subst
        }
      },

      _ => panic!("failed to unify: {} and {}", t1, t2),
    }
  }

  fn compose_substitutions(
    &mut self,
    s1: &TypeSubstitution,
    s2: &TypeSubstitution,
  ) -> TypeSubstitution {
    let mut composed = HashMap::new();

    // first, add all entries in s1
    for (k, v) in s1 {
      composed.insert(*k, v.clone());
    }

    // then, add all entries in s2, but apply s1 to them
    for (k, v) in s2 {
      composed.insert(*k, self.substitute_in_type(s1, v));
    }

    composed
  }

  fn substitute_in_type(&mut self, substitution: &TypeSubstitution, ty: &Type) -> Type {
    match ty {
      Type::Int | Type::String => ty.clone(),

      Type::Var(n) => match substitution.get(n) {
        Some(replacement_ty) => replacement_ty.clone(),
        _ => ty.clone(),
      },

      Type::Fun(param_types, return_type) => {
        let mut substituted_param_types = Vec::new();

        for param_type in param_types {
          substituted_param_types.push(self.substitute_in_type(substitution, param_type));
        }

        let substituted_return_type = self.substitute_in_type(substitution, return_type);

        Type::Fun(substituted_param_types, substituted_return_type.into())
      }
    }
  }

  fn substitute_in_context(
    &mut self,
    substitution: &TypeSubstitution,
    ctx: &TypeContext,
  ) -> TypeContext {
    let mut substituted_ctx = ctx.clone();

    for (name, scheme) in ctx {
      substituted_ctx.insert(
        name.clone(),
        self.substitute_in_scheme(&substitution, scheme),
      );
    }

    substituted_ctx
  }

  fn substitute_in_scheme(
    &mut self,
    substitution: &TypeSubstitution,
    scheme: &TypeScheme,
  ) -> TypeScheme {
    match scheme {
      TypeScheme::Mono(_) => scheme.clone(),
      TypeScheme::Poly(forall_vars, ty) => {
        let mut substitution_without_forall_vars = substitution.clone();

        for var in forall_vars {
          substitution_without_forall_vars.remove(&var);
        }

        TypeScheme::Poly(
          forall_vars.clone(),
          self.substitute_in_type(&substitution_without_forall_vars, ty),
        )
      }
    }
  }

  fn new_type_var(&mut self) -> Type {
    let type_var = Type::Var(self.next_placeholder_id);
    self.next_placeholder_id += 1;
    type_var
  }

  #[allow(unused)] // when to call this?
  fn generalize(&mut self, ctx: &TypeContext, ty: &Type) -> TypeScheme {
    let mut forall_vars = Vec::new();

    // find all vars that are free in ty, but not free in ctx
    let ctx_free_vars = self.free_type_vars_in_context(ctx);
    for var in self.free_type_vars(ty) {
      if !ctx_free_vars.contains(&var) {
        forall_vars.push(var);
      }
    }

    if forall_vars.is_empty() {
      TypeScheme::Mono(ty.clone())
    } else {
      TypeScheme::Poly(forall_vars, ty.clone())
    }
  }

  fn free_type_vars(&mut self, ty: &Type) -> HashSet<usize> {
    match ty {
      Type::Var(var) => HashSet::from([*var]),

      Type::Fun(param_types, return_type) => {
        let mut set = HashSet::new();

        for param_type in param_types {
          for var in self.free_type_vars(param_type) {
            set.insert(var);
          }
        }

        for var in self.free_type_vars(return_type) {
          set.insert(var);
        }

        set
      }

      _ => HashSet::new(),
    }
  }

  fn free_type_vars_in_scheme(&mut self, scheme: &TypeScheme) -> HashSet<usize> {
    match scheme {
      TypeScheme::Mono(_) => HashSet::new(),
      TypeScheme::Poly(forall_vars, ty) => {
        // find all free vars in ty, then remove the ones listed in forall vars
        // (because these are not really free! they are bound by the quantifier)
        let mut set = self.free_type_vars(ty);
        for var in forall_vars {
          set.remove(var);
        }
        set
      }
    }
  }

  fn free_type_vars_in_context(&mut self, ctx: &TypeContext) -> HashSet<usize> {
    let mut set = HashSet::new();
    for (_, scheme) in ctx {
      for var in self.free_type_vars_in_scheme(scheme) {
        set.insert(var);
      }
    }
    set
  }

  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////
  /////

  pub fn analyze2(&mut self, module: &mut Module) {
    self.module_name = Some(module.module_name.clone());
    self.module_path = Some(module.module_path.clone());

    if let Some(ast) = &mut module.ast {
      self.annotate_with_placeholders(ast);
      println!("ast with placeholders: {:#?}", ast);

      let constraints = self.generate_constraints(ast);
      for c in &constraints {
        if let Eq(a, b) = c {
          println!("{} :: {}", a, b);
        }
      }

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
      _other => {
        println!("other kind of expr")
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
        constraints.push(Eq(
          definition.inferred_type.clone(),
          expr.inferred_type.clone(),
        ));
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
        constraints.push(Eq(inferred_type, ExprType::Regex));
      }

      ExprKind::Grouping(inner) => {
        self.constraints_from_expr(inner, constraints);
        constraints.push(Eq(inferred_type, inner.inferred_type.clone()));
      }

      ExprKind::BinaryOperation { left, right, op } => {
        self.constraints_from_expr(left, constraints);
        self.constraints_from_expr(right, constraints);

        match op.kind {
          Operator::Addition => {
            // todo: floats?
            constraints.push(Eq(left.inferred_type.clone(), ExprType::Int));
            constraints.push(Eq(right.inferred_type.clone(), ExprType::Int));
            constraints.push(Eq(inferred_type.clone(), ExprType::Int));
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
        constraints.push(Eq(
          inferred_type,
          ExprType::Func(param_types, Box::new(return_type)),
        ));
      }

      ExprKind::Call(CallNode { callee, args, .. }) => {
        let arg_types: Vec<ExprType> = args.iter().map(|a| a.inferred_type.clone()).collect();

        self.constraints_from_expr(callee, constraints);

        for arg in args {
          self.constraints_from_expr(arg, constraints);
        }

        // we know that the callee should be a function that takes
        // the given arg types and returns the type of this whole expr
        // TODO: i don't think this is quite right, since it doesn't
        // allow for (e.g.) the identify function to be used on diff types
        println!(
          "HERE: adding {} :: {}",
          callee.inferred_type,
          ExprType::Func(arg_types.clone(), Box::new(inferred_type.clone()))
        );
        constraints.push(Eq(
          callee.inferred_type.clone(),
          ExprType::Func(arg_types, Box::new(inferred_type)),
        ));
      }

      ExprKind::Let(LetNode { value, .. }) => {
        self.constraints_from_expr(value, constraints);

        // let expressions always evaluate to ()
        constraints.push(Eq(inferred_type, ExprType::Nothing));
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
        constraints.push(Eq(typ, ExprType::String));
      }

      LiteralKind::FloatDecimal(..) => {
        constraints.push(Eq(typ, ExprType::Float));
      }

      LiteralKind::IntDecimal(..)
      | LiteralKind::IntHex(..)
      | LiteralKind::IntBinary(..)
      | LiteralKind::IntOctal(..) => {
        constraints.push(Eq(typ, ExprType::Int));
      }
    }
  }
}

// Constraint-solving methods
impl<'compiler> Analyzer<'compiler> {
  fn unify_constraints(&mut self, constraints: &ConstraintSet) -> SolutionMap {
    if constraints.is_empty() {
      return SolutionMap::empty();
    }

    let solutions_for_head = self.unify_constraint(&constraints[0]);
    let new_tail_constraints = solutions_for_head.apply_to_constraints(&constraints[1..]);
    let solutions_for_tail = self.unify_constraints(&new_tail_constraints);
    solutions_for_head.compose(solutions_for_tail)
  }

  fn unify_constraint(&mut self, constraint: &Constraint) -> SolutionMap {
    match constraint {
      // same types, and both are "leaf" nodes; nothing to add to the solution
      Eq(t1, t2) if t1 == t2 && !t1.has_any_placeholders() && !t2.has_any_placeholders() => {
        SolutionMap::empty()
      }

      Eq(
        ExprType::Func(param_types_1, return_type_1),
        ExprType::Func(param_types_2, return_type_2),
      ) => {
        // add some new constraints to unify param & return types:
        let mut constraints = Vec::with_capacity(param_types_1.len() + 1);

        for i in 0..param_types_1.len() {
          constraints.push(Eq(param_types_1[i].clone(), param_types_2[i].clone()))
        }

        constraints.push(Eq(*return_type_1.clone(), *return_type_2.clone()));

        self.unify_constraints(&constraints)
      }

      Eq(ExprType::Placeholder(n), t) | Eq(t, ExprType::Placeholder(n)) => match t {
        ExprType::Placeholder(n2) if n == n2 => SolutionMap::empty(),
        ExprType::Placeholder(_) => SolutionMap::with_entry(*n, t.clone()),
        other => {
          if other.contains_placeholder(n) {
            todo!("circular reference! can't unify!")
          }

          SolutionMap::with_entry(*n, t.clone())
        }
      },

      Eq(a, b) => {
        // self.error(
        //   *span,
        //   TypeMismatch {
        //     expected: a.clone(),
        //     found: b.clone(),
        //   },
        // );
        panic!("failed to unify {} and {}", a, b);

        // SolutionMap::empty()
      }

      _ => {
        // ???
        todo!()
      }
    }
  }
}

// Decoration methods
impl<'compiler> Analyzer<'compiler> {
  fn fill_in_placeholder(&mut self, ty: &mut ExprType, solutions: &SolutionMap) {
    if let ExprType::Placeholder(n) = ty {
      if let Some(actual_type) = solutions.solutions.get(&n) {
        *ty = actual_type.clone();
      }
    }
  }

  fn decorate_with_inferred_types(&mut self, module: &mut ModuleNode, solutions: &SolutionMap) {
    for definition in &mut module.body {
      self.decorate_definition(definition, solutions)
    }
  }

  fn decorate_definition(&mut self, definition: &mut DefinitionNode, solutions: &SolutionMap) {
    self.fill_in_placeholder(&mut definition.inferred_type, solutions);

    println!("{} :: {}", definition.name.name, definition.inferred_type);

    match &mut definition.kind {
      DefinitionKind::Expr(expr) => self.decorate_expr(expr, solutions),
      _ => { /* todo */ }
    }
  }

  fn decorate_expr(&mut self, expr: &mut ExprNode, solutions: &SolutionMap) {
    self.fill_in_placeholder(&mut expr.inferred_type, solutions);

    match &mut expr.kind {
      ExprKind::Lambda(LambdaNode { params, body, .. }) => {
        for param in params {
          self.fill_in_placeholder(&mut param.inferred_type, solutions);
        }

        for expr in body {
          self.fill_in_placeholder(&mut expr.inferred_type, solutions);
        }
      }

      ExprKind::Call(CallNode { callee, args, .. }) => {
        self.fill_in_placeholder(&mut callee.inferred_type, solutions);

        for expr in args {
          self.fill_in_placeholder(&mut expr.inferred_type, solutions);
        }
      }

      _ => {}
    }
  }
}
