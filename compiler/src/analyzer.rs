use crate::ast::*;
use crate::binding::*;
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
  value_scopes: Vec<HashMap<String, ValueBinding>>,
  next_placeholder_id: usize,
}

impl<'compiler> Analyzer<'compiler> {
  pub fn new(diagnostics: &'compiler mut Vec<Diagnostic>) -> Self {
    Self {
      module_name: None,
      module_path: None,
      diagnostics,
      value_scopes: Vec::new(),
      next_placeholder_id: 0,
    }
  }

  pub fn analyze(&mut self, module: &mut Module) {
    self.module_name = Some(module.module_name.clone());
    self.module_path = Some(module.module_path.clone());

    // initialize top-level scope
    self.enter_scope();

    if let Some(ast) = &mut module.ast {
      // self.annotate_with_placeholders(ast);

      println!("ast: {:#?}", ast);

      let constraints = self.constrain(ast);

      println!("annotated: {:#?}", ast);

      for c in &constraints {
        println!("{:?}", c);
      }

      let substitution = self.unify_constraints(&constraints);

      println!("=========");
      self.decorate_with_inferred_types(ast, &substitution);
    }
  }

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

  fn add_value_binding(&mut self, name: String, ty_scheme: TypeScheme, span: (usize, usize)) {
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
        if binding.ref_count == 0 {
          self.warning(binding.span, UnusedBinding { name });
        }
      }
    }
  }

  fn constrain(&mut self, module: &mut ModuleNode) -> Vec<TypeConstraint> {
    let mut constraints = Vec::new();

    // we first do a shallow pass to annotate all top-level defs and add them to the scope,
    // so that they can be referenced anywhere within the bodies of other defs
    for definition in &mut module.body {
      definition.ty = self.new_type_var();

      match &mut definition.kind {
        DefinitionKind::Expr(_) => self.add_value_binding(
          definition.name.name.clone(),
          TypeScheme::Forall(vec![], definition.ty.clone()),
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

          constraints.push(TypeConstraint::Eq(definition.ty.clone(), expr.ty.clone()));
        }

        _ => {
          // todo :---)
        }
      }
    }

    constraints
  }

  fn constrain_expr(&mut self, expr: &mut ExprNode, constraints: &mut Vec<TypeConstraint>) {
    use TypeConstraint::*;

    let expr_ty = self.new_type_var();

    expr.ty = expr_ty.clone();

    match &mut expr.kind {
      ExprKind::Identifier(ident) => {
        if let Some(binding) = self.get_value_binding(&ident.name) {
          match &binding.ty_scheme {
            TypeScheme::Forall(_, ty) => constraints.push(Eq(expr_ty, ty.clone())),
            TypeScheme::Var(var) => constraints.push(Inst(*var, expr_ty)),
          }
        } else {
          self.error(
            ident.span,
            NameNotBound {
              name: ident.name.clone(),
            },
          );

          constraints.push(Eq(expr_ty, Type::Unknown));
        }
      }

      ExprKind::Literal(literal) => match &mut literal.kind {
        LiteralKind::Str(..) => {
          constraints.push(Eq(expr_ty, Type::String));
        }

        LiteralKind::FloatDecimal(..) => {
          constraints.push(Eq(expr_ty, Type::Float));
        }

        LiteralKind::IntDecimal(..)
        | LiteralKind::IntHex(..)
        | LiteralKind::IntBinary(..)
        | LiteralKind::IntOctal(..) => {
          constraints.push(Eq(expr_ty, Type::Int));
        }
      },

      ExprKind::Regex(..) => {
        constraints.push(Eq(expr_ty, Type::Regex));
      }

      ExprKind::Grouping(inner) => {
        self.constrain_expr(inner, constraints);

        constraints.push(Eq(expr_ty, inner.ty.clone()));
      }

      ExprKind::BinaryOperation { left, right, op } => {
        self.constrain_expr(left, constraints);
        self.constrain_expr(right, constraints);

        match op.kind {
          Operator::Addition => {
            // todo: floats?
            constraints.push(Eq(left.ty.clone(), Type::Int));
            constraints.push(Eq(right.ty.clone(), Type::Int));
            constraints.push(Eq(expr_ty.clone(), Type::Int));
          }
          _ => {
            // todo :----)
          }
        }
      }

      ExprKind::Lambda(LambdaNode { params, body, .. }) => {
        let mut param_types = Vec::new();

        self.enter_scope();

        // TODO: lambdas with 0 params?

        for param in params {
          param.ty = self.new_type_var();

          param_types.push(param.ty.clone());

          self.add_value_binding(
            param.ident.name.clone(),
            TypeScheme::Forall(vec![], param.ty.clone()),
            param.ident.span,
          )
        }

        let mut return_type = Type::Nothing;

        for expr in body {
          self.constrain_expr(expr, constraints);
          return_type = expr.ty.clone();
        }

        self.leave_scope();

        // we know that this lambda must be a function that takes
        // the param types and returns the return type
        constraints.push(Eq(expr_ty, Type::Fun(param_types, Box::new(return_type))));
      }

      ExprKind::Call(CallNode { callee, args, .. }) => {
        self.constrain_expr(callee, constraints);

        let mut arg_types = Vec::new();

        for arg in args {
          self.constrain_expr(arg, constraints);
          arg_types.push(arg.ty.clone());
        }

        // we know that the callee should be a function that takes
        // the given arg types and returns the type of this whole expr
        constraints.push(Eq(callee.ty.clone(), Type::Fun(arg_types, expr_ty.into())));
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
        constraints.push(Eq(expr_ty, Type::Nothing));
      }

      _ => {
        // todo :---)
      }
    }
  }

  fn unify_constraints(&mut self, constraints: &[TypeConstraint]) -> TypeSubstitution {
    if constraints.is_empty() {
      return TypeSubstitution::empty();
    }

    let solutions_for_head = self.unify_constraint(&constraints[0]);
    let new_tail_constraints = solutions_for_head.apply_to_constraints(&constraints[1..]);
    let solutions_for_tail = self.unify_constraints(&new_tail_constraints);
    solutions_for_head.compose(solutions_for_tail)
  }

  fn unify_constraint(&mut self, constraint: &TypeConstraint) -> TypeSubstitution {
    use TypeConstraint::*;

    match constraint {
      // same types, and both are "leaf" nodes; nothing to add to the solution
      Eq(Type::Int, Type::Int)
      | Eq(Type::Float, Type::Float)
      | Eq(Type::String, Type::String)
      | Eq(Type::Regex, Type::Regex)
      | Eq(Type::Nothing, Type::Nothing) => TypeSubstitution::empty(),

      Eq(Type::Fun(param_types_1, return_type_1), Type::Fun(param_types_2, return_type_2)) => {
        // add some new constraints to unify param & return types:
        let mut constraints = Vec::with_capacity(param_types_1.len() + 1);

        for i in 0..param_types_1.len() {
          constraints.push(Eq(param_types_1[i].clone(), param_types_2[i].clone()))
        }

        constraints.push(Eq(*return_type_1.clone(), *return_type_2.clone()));

        self.unify_constraints(&constraints)
      }

      Eq(Type::Var(n), t) | Eq(t, Type::Var(n)) => match t {
        Type::Var(n2) if n == n2 => TypeSubstitution::empty(),
        Type::Var(_) => TypeSubstitution::with_entry(*n, t.clone()),
        other => {
          if other.contains_var(*n) {
            self.error((0, 0), RecursiveUnification { ty: other.clone() });
            return TypeSubstitution::empty();
          }

          TypeSubstitution::with_entry(*n, t.clone())
        }
      },

      Eq(a, b) => {
        self.error(
          (0, 0),
          TypeMismatch {
            expected: b.clone(),
            found: a.clone(),
          },
        );

        TypeSubstitution::empty()
      }

      _ => {
        // ???
        todo!()
      }
    }
  }

  fn fill_in_placeholder(&mut self, ty: &mut Type, subst: &TypeSubstitution) {
    if let Type::Var(n) = ty {
      if let Some(actual_type) = subst.solutions.get(&n) {
        *ty = actual_type.clone();
      }
    }
  }

  fn decorate_with_inferred_types(&mut self, module: &mut ModuleNode, subst: &TypeSubstitution) {
    for definition in &mut module.body {
      self.decorate_definition(definition, subst)
    }
  }

  fn decorate_definition(&mut self, definition: &mut DefinitionNode, subst: &TypeSubstitution) {
    self.fill_in_placeholder(&mut definition.ty, subst);

    println!("{} :: {}", definition.name.name, definition.ty);

    match &mut definition.kind {
      DefinitionKind::Expr(expr) => self.decorate_expr(expr, subst),
      _ => { /* todo */ }
    }
  }

  fn decorate_expr(&mut self, expr: &mut ExprNode, subst: &TypeSubstitution) {
    self.fill_in_placeholder(&mut expr.ty, subst);

    match &mut expr.kind {
      ExprKind::Lambda(LambdaNode { params, body, .. }) => {
        for param in params {
          self.fill_in_placeholder(&mut param.ty, subst);
        }

        for expr in body {
          self.fill_in_placeholder(&mut expr.ty, subst);
        }
      }

      ExprKind::Call(CallNode { callee, args, .. }) => {
        self.fill_in_placeholder(&mut callee.ty, subst);

        for expr in args {
          self.fill_in_placeholder(&mut expr.ty, subst);
        }
      }

      _ => {}
    }
  }

  // fn unify(&mut self, constraints: &[TypeConstraint]) -> TypeSubstitution {
  //   let mut eq_constraints = Vec::new();
  //   let mut other_constraints = Vec::new();

  //   for constraint in constraints {
  //     if let TypeConstraint::Eq(..) = constraint {
  //       eq_constraints.push(constraint.clone())
  //     } else {
  //       other_constraints.push(constraint.clone())
  //     }
  //   }

  //   let subst1 = self.unify_eq(&eq_constraints);
  //   let other_constraints = self.substitute_constraints(&other_constraints, &subst1);
  //   let subst2 = self.unify_gen_inst(&other_constraints);

  //   self.compose_substitutions(&subst1, &subst2)
  // }

  // fn unify_eq(&mut self, constraints: &[TypeConstraint]) -> TypeSubstitution {
  //   if constraints.is_empty() {
  //     return HashMap::new();
  //   }

  //   match constraints.get(0).unwrap() {
  //     TypeConstraint::Eq(ty1, ty2) => match (ty1, ty2) {
  //       (Type::Int, Type::Int) => self.unify_eq(&constraints[1..]),
  //       (Type::Float, Type::Float) => self.unify_eq(&constraints[1..]),
  //       (Type::String, Type::String) => self.unify_eq(&constraints[1..]),
  //       (Type::Regex, Type::Regex) => self.unify_eq(&constraints[1..]),
  //       (Type::Unknown, Type::Unknown) => self.unify_eq(&constraints[1..]),
  //       (Type::Nothing, Type::Nothing) => self.unify_eq(&constraints[1..]),

  //       (Type::Var(var), ty) | (ty, Type::Var(var)) => match ty {
  //         Type::Var(var2) if var == var2 => self.unify_eq(&constraints[1..]),
  //         _ => {
  //           if self.free_type_vars(ty).contains(var) {
  //             self.error((0, 0), RecursiveUnification { ty: ty.clone() });
  //             return self.unify_eq(&constraints[1..]);
  //           }

  //           let mut new_subst = HashMap::new();
  //           new_subst.insert(*var, ty.clone());

  //           let rest_constraints = &self.substitute_constraints(&constraints[1..], &new_subst);
  //           let rest_constraints = &self.unify_eq(rest_constraints);
  //           self.compose_substitutions(&new_subst, rest_constraints)
  //         }
  //       },

  //       (Type::Fun(params_1, return_1), Type::Fun(params_2, return_2)) => {
  //         let mut new_constraints = Vec::new();

  //         // TODO: ensure same # of params in both

  //         // to unify functions, we generate some new constraints to make sure each
  //         // param and return type match, then continue with the rest of the constraints

  //         for i in 0..params_1.len() {
  //           new_constraints.push(TypeConstraint::Eq(params_1[i].clone(), params_2[i].clone()));
  //         }

  //         new_constraints.push(TypeConstraint::Eq(*return_1.clone(), *return_2.clone()));

  //         for existing_constraint in constraints {
  //           new_constraints.push(existing_constraint.clone())
  //         }

  //         self.unify_eq(&new_constraints)
  //       }

  //       (t1, t2) => panic!("failed to unify {} and {}", t1, t2),
  //     },

  //     _ => unreachable!(),
  //   }
  // }

  // fn unify_gen_inst(&mut self, constraints: &Vec<TypeConstraint>) -> TypeSubstitution {
  //   if constraints.is_empty() {
  //     return HashMap::new();
  //   }

  //   match constraints.get(0).unwrap() {
  //     TypeConstraint::Gen(scheme, ty) => {
  //       let mut inst_constraints = Vec::new();
  //       let mut other_constraints = Vec::new();

  //       for constraint in &constraints[1..] {
  //         match (constraint, scheme) {
  //           // hmm, will the vars ever match?
  //           (TypeConstraint::Inst(var1, ..), TypeScheme::Var(var2, ..)) if var1 == var2 => {
  //             inst_constraints.push(constraint.clone())
  //           }
  //           _ => other_constraints.push(constraint.clone()),
  //         }
  //       }

  //       let new_eq_constraints = self.instantiate_constraints(&inst_constraints, ty);
  //       let subst = self.unify_eq(&new_eq_constraints);
  //       let other_constraints = self.substitute_constraints(&other_constraints, &subst);

  //       let subst2 = &self.unify_gen_inst(&other_constraints);
  //       self.compose_substitutions(&subst, subst2)
  //     }

  //     _ => unreachable!("should have a Gen first"),
  //   }
  // }

  // fn instantiate_constraints(
  //   &mut self,
  //   constraints: &[TypeConstraint],
  //   ty: &Type,
  // ) -> Vec<TypeConstraint> {
  //   let mut new_constraints = Vec::new();

  //   let scheme = self.generalize(ty);

  //   for constraint in constraints {
  //     if let TypeConstraint::Inst(_, ty) = constraint {
  //       let inst_ty = self.instantiate_scheme(&scheme);
  //       new_constraints.push(TypeConstraint::Eq(ty.clone(), inst_ty));
  //     } else {
  //       unreachable!("should only have Insts here");
  //     }
  //   }

  //   new_constraints
  // }

  // fn substitute_constraints(
  //   &mut self,
  //   constraints: &[TypeConstraint],
  //   subst: &TypeSubstitution,
  // ) -> Vec<TypeConstraint> {
  //   let mut new_constraints = Vec::new();

  //   for constraint in constraints {
  //     match constraint {
  //       TypeConstraint::Eq(ty1, ty2) => new_constraints.push(TypeConstraint::Eq(
  //         self.substitute_in_type(subst, &ty1),
  //         self.substitute_in_type(subst, &ty2),
  //       )),
  //       // TODO: should we have a context arg here as well?
  //       // see https://github.com/igstan/linguae/blob/7e806dd121c21ed35187377fe3bd92d29d6150e6/lingua-002-hm-inference-sml/src/constraint.sml#L21
  //       TypeConstraint::Gen(scheme, ty) => new_constraints.push(TypeConstraint::Gen(
  //         scheme.clone(),
  //         self.substitute_in_type(subst, &ty),
  //       )),
  //       TypeConstraint::Inst(var, ty) => new_constraints.push(TypeConstraint::Inst(
  //         *var,
  //         self.substitute_in_type(subst, &ty),
  //       )),
  //     }
  //   }

  //   new_constraints
  // }

  fn new_type_scheme_var(&mut self) -> TypeScheme {
    let type_var = TypeScheme::Var(self.next_placeholder_id);
    self.next_placeholder_id += 1;
    type_var
  }

  fn new_type_var(&mut self) -> Type {
    let type_var = Type::Var(self.next_placeholder_id);
    self.next_placeholder_id += 1;
    type_var
  }

  fn free_type_vars(&self, ty: &Type) -> HashSet<usize> {
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

  // #[allow(unused)]
  // pub fn analyze2(&mut self, module: &mut Module) {
  //   self.module_name = Some(module.module_name.clone());
  //   self.module_path = Some(module.module_path.clone());

  //   let initial_ctx = TypeContext::empty();

  //   if let Some(ast) = &mut module.ast {
  //     for definition in &mut ast.body {
  //       if let DefinitionKind::Expr(expr) = &mut definition.kind {
  //         let (substitution, ty) = self.infer_expr(&initial_ctx, expr);
  //         let inferred_type = self.substitute_in_type(&substitution, &ty);
  //         println!("{} :: {}", definition.name.name, inferred_type);
  //         println!("    before subst: {}", ty);
  //       }
  //     }
  //   }
  // }

  // fn infer_expr(&mut self, ctx: &TypeContext, expr: &ExprNode) -> (TypeSubstitution, Type) {
  //   match &expr.kind {
  //     ExprKind::Literal(literal) => self.infer_literal(&literal),

  //     ExprKind::Grouping(inner) => self.infer_expr(ctx, inner),

  //     ExprKind::Identifier(ident) => match ctx.get(&ident.name) {
  //       Some(scheme) => {
  //         let ty = self.instantiate(scheme);
  //         (HashMap::new(), ty)
  //       }

  //       None => {
  //         self.error(
  //           ident.span,
  //           NameNotBound {
  //             name: ident.name.clone(),
  //           },
  //         );

  //         (HashMap::new(), Type::Unknown)
  //       }
  //     },

  //     ExprKind::Call(CallNode { callee, args, .. }) => {
  //       let return_type = self.new_type_var();

  //       let (s1, callee_type) = self.infer_expr(&ctx, &callee);

  //       // TODO: support multiple args
  //       let arg = &args[0];
  //       let arg_ctx = self.substitute_in_context(&s1, &ctx);
  //       let (s2, arg_type) = self.infer_expr(&arg_ctx, &arg);

  //       let inferred_callee_type = self.substitute_in_type(&s2, &callee_type);
  //       let expected_callee_type = Type::Fun(vec![arg_type], return_type.clone().into());

  //       let s3 = self.unify(&inferred_callee_type, &expected_callee_type);

  //       let composed_subst = self.compose_substitutions(&s3, &s2);
  //       let composed_subst = self.compose_substitutions(&composed_subst, &s1);

  //       (composed_subst, self.substitute_in_type(&s3, &return_type))
  //     }

  //     ExprKind::Lambda(LambdaNode { params, body, .. }) => {
  //       let mut lambda_ctx = ctx.clone();
  //       let mut param_types = Vec::new();

  //       for param in params {
  //         // within this lambda scope, extend ctx to include params
  //         let param_type = self.new_type_var();
  //         param_types.push(param_type.clone());
  //         let param_type_scheme = TypeScheme::Mono(param_type.clone());
  //         lambda_ctx.insert(param.ident.name.clone(), param_type_scheme);
  //       }

  //       // TODO: support multiple body exprs
  //       let body = body.get(0).unwrap();

  //       let (s1, body_type) = self.infer_expr(&lambda_ctx, &body);

  //       (s1, Type::Fun(param_types, body_type.into()))
  //     }

  //     other => todo!("other expr kind: {:#?}", other),
  //   }
  // }

  // fn infer_literal(&mut self, literal: &LiteralNode) -> (TypeSubstitution, Type) {
  //   match literal.kind {
  //     LiteralKind::IntDecimal(..) => (HashMap::new(), Type::Int),
  //     LiteralKind::Str(..) => (HashMap::new(), Type::String),
  //     _ => todo!("more literal kinds!"),
  //   }
  // }

  // fn instantiate_scheme(&mut self, scheme: &TypeScheme) -> Type {
  //   match scheme {
  //     TypeScheme::Var(_) => unreachable!("can this happen?"),
  //     // TypeScheme::Var(ty) => Type::Var(*ty),
  //     TypeScheme::Forall(vars, ty) => {
  //       println!("INSTANTIATING A SCHEME!!!");

  //       // create a substitution that replaces all forall vars with new vars
  //       let mut subst = HashMap::new();
  //       for var in vars {
  //         subst.insert(*var, self.new_type_var());
  //       }

  //       // ...and apply it
  //       self.substitute_in_type(&subst, ty)
  //     }
  //   }
  // }

  // fn compose_substitutions(
  //   &mut self,
  //   s1: &TypeSubstitution,
  //   s2: &TypeSubstitution,
  // ) -> TypeSubstitution {
  //   let mut composed = HashMap::new();

  //   // first, add all entries in s1
  //   for (k, v) in s1 {
  //     composed.insert(*k, v.clone());
  //   }

  //   // then, add all entries in s2, but apply s1 to them
  //   for (k, v) in s2 {
  //     composed.insert(*k, self.substitute_in_type(s1, v));
  //   }

  //   composed
  // }

  // fn substitute_in_type(&mut self, substitution: &TypeSubstitution, ty: &Type) -> Type {
  //   match ty {
  //     Type::Var(n) => match substitution.get(n) {
  //       Some(replacement_ty) => replacement_ty.clone(),
  //       _ => ty.clone(),
  //     },

  //     Type::Fun(param_types, return_type) => {
  //       let mut substituted_param_types = Vec::new();

  //       for param_type in param_types {
  //         substituted_param_types.push(self.substitute_in_type(substitution, param_type));
  //       }

  //       let substituted_return_type = self.substitute_in_type(substitution, return_type);

  //       Type::Fun(substituted_param_types, substituted_return_type.into())
  //     }

  //     _ => ty.clone(),
  //   }
  // }

  // fn substitute_in_context(
  //   &mut self,
  //   substitution: &TypeSubstitution,
  //   ctx: &TypeContext,
  // ) -> TypeContext {
  //   let mut substituted_ctx = ctx.clone();

  //   // for (name, scheme) in ctx {
  //   //   substituted_ctx.insert(
  //   //     name.clone(),
  //   //     self.substitute_in_scheme(&substitution, scheme),
  //   //   );
  //   // }

  //   substituted_ctx
  // }

  // fn substitute_in_scheme(
  //   &mut self,
  //   substitution: &TypeSubstitution,
  //   scheme: &TypeScheme,
  // ) -> TypeScheme {
  //   match scheme {
  //     TypeScheme::Mono(_) => scheme.clone(),
  //     TypeScheme::Poly(forall_vars, ty) => {
  //       let mut substitution_without_forall_vars = substitution.clone();

  //       for var in forall_vars {
  //         substitution_without_forall_vars.remove(&var);
  //       }

  //       TypeScheme::Poly(
  //         forall_vars.clone(),
  //         self.substitute_in_type(&substitution_without_forall_vars, ty),
  //       )
  //     }
  //   }
  // }

  fn generalize(&mut self, ty: &Type) -> TypeScheme {
    let mut vars = Vec::new();

    // find all vars that are free in ty, but not free in scope
    let ctx_free_vars = self.free_type_vars_in_scope();
    for var in self.free_type_vars(ty) {
      if !ctx_free_vars.contains(&var) {
        vars.push(var);
      }
    }

    TypeScheme::Forall(vars, ty.clone())
  }

  fn free_type_vars_in_scheme(&self, scheme: &TypeScheme) -> HashSet<usize> {
    match scheme {
      TypeScheme::Var(_) => HashSet::new(),
      TypeScheme::Forall(vars, ty) => {
        // find all free vars in ty, then remove the ones listed in forall vars
        // (because these are not really free! they are bound by the quantifier)
        let mut set = self.free_type_vars(ty);
        for var in vars {
          set.remove(var);
        }
        set
      }
    }
  }

  fn free_type_vars_in_scope(&self) -> HashSet<usize> {
    let mut set = HashSet::new();

    // todo: more than just last scope?
    for (_, binding) in self.value_scopes.last().unwrap() {
      for var in self.free_type_vars_in_scheme(&binding.ty_scheme) {
        set.insert(var);
      }
    }

    set
  }
}
