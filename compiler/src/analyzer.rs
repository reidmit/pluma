use crate::ast::*;
use crate::diagnostic::*;
use crate::errors::*;
use crate::module::Module;
use crate::typing::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use AnalysisErrorKind::*;

pub struct Analyzer<'compiler> {
  module_name: Option<String>,
  module_path: Option<PathBuf>,
  diagnostics: &'compiler mut Vec<Diagnostic>,
  next_placeholder_id: usize,
}

// Public interface
impl<'compiler> Analyzer<'compiler> {
  pub fn new(diagnostics: &'compiler mut Vec<Diagnostic>) -> Self {
    Self {
      module_name: None,
      module_path: None,
      diagnostics,
      next_placeholder_id: 0,
    }
  }

  pub fn analyze(&mut self, module: &mut Module) {
    self.module_name = Some(module.module_name.clone());
    self.module_path = Some(module.module_path.clone());

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

  fn diagnostic(&mut self, span: (usize, usize), diag: Diagnostic) {
    let mut diag = diag.with_pos(span);

    if let Some(module_name) = &self.module_name {
      diag = diag.with_module(module_name.clone(), self.module_path.clone().unwrap())
    }

    self.diagnostics.push(diag)
  }

  // fn warning(&mut self, span: (usize, usize), kind: AnalysisErrorKind) {
  //   self.diagnostic(span, Diagnostic::warning(AnalysisError { span, kind }));
  // }

  fn error(&mut self, span: (usize, usize), kind: AnalysisErrorKind) {
    self.diagnostic(span, Diagnostic::error(AnalysisError { span, kind }));
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

        None => {
          self.error(
            ident.span,
            NameNotBound {
              name: ident.name.clone(),
            },
          );

          (HashMap::new(), Type::Unknown)
        }
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

          let param_type_scheme = TypeScheme::Mono(param_type.clone());

          // within this lambda scope, extend ctx to include params
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
          if self.free_type_vars(ty).contains(var) {
            self.error((0, 0), RecursiveUnification { ty: ty.clone() });
            return HashMap::new();
          }

          let mut subst = HashMap::new();
          subst.insert(*var, ty.clone());
          subst
        }
      },

      _ => {
        self.error(
          (0, 0), // todo: real span
          TypeMismatch {
            expected: t1.clone(),
            found: t2.clone(),
          },
        );

        HashMap::new()
      }
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

      _ => ty.clone(),
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
}
