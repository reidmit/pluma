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

pub struct Analyzer<'compiler> {
  module_name: Option<String>,
  module_path: Option<PathBuf>,
  diagnostics: &'compiler mut Vec<Diagnostic>,
  type_scope: HashMap<String, TypeBinding>,
  value_scopes: Vec<HashMap<String, ValueBinding>>,
  next_placeholder_id: usize,
  constraints: HashSet<(ExprType, ExprType)>,
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
      constraints: HashSet::new(),
    }
  }

  pub fn analyze(&mut self, module: &mut Module) {
    self.module_name = Some(module.module_name.clone());
    self.module_path = Some(module.module_path.clone());

    if let Some(ast) = &mut module.ast {
      // phase 1: annotation
      self.annotate(ast);

      // phase 2: constraint generation
      self.generate_constraints(ast);

      println!("AFTER: {:#?}", ast);
      println!("CONSTRAINTS:");
      for c in &self.constraints {
        println!("{} :: {}", c.0, c.1)
      }
    }
  }
}

// Helper methods
impl<'compiler> Analyzer<'compiler> {
  fn diagnostic(&mut self, pos: (usize, usize), diag: Diagnostic) {
    let mut diag = diag.with_pos(pos);

    if let Some(module_name) = &self.module_name {
      diag = diag.with_module(module_name.clone(), self.module_path.clone().unwrap())
    }

    self.diagnostics.push(diag)
  }

  fn warning(&mut self, pos: (usize, usize), kind: AnalysisErrorKind) {
    self.diagnostic(pos, Diagnostic::warning(AnalysisError { pos, kind }));
  }

  fn error(&mut self, pos: (usize, usize), kind: AnalysisErrorKind) {
    self.diagnostic(pos, Diagnostic::error(AnalysisError { pos, kind }));
  }

  fn enter_scope(&mut self) {
    self.value_scopes.push(HashMap::new());
  }

  pub fn leave_scope(&mut self) {
    if let Some(exited_level) = self.value_scopes.pop() {
      for (name, binding) in exited_level {
        if binding.ref_count == 0 {
          self.warning(binding.pos, UnusedBinding { name });
        }
      }
    }
  }

  fn new_placeholder_type(&mut self) -> ExprType {
    let placeholder_id = self.next_placeholder_id;
    self.next_placeholder_id += 1;
    ExprType::Placeholder(placeholder_id)
  }

  fn add_value_binding(&mut self, name: String, typ: ExprType, pos: (usize, usize)) {
    let current_level = self.value_scopes.last_mut().expect("no current scope");

    current_level.insert(
      name,
      ValueBinding {
        typ,
        ref_count: 0,
        pos,
      },
    );
  }

  pub fn add_type_binding(&mut self, name: String, typ: ExprType, pos: (usize, usize)) {
    self.type_scope.insert(name, TypeBinding { typ, pos });
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

  fn destructure_pattern(&mut self, pattern: &mut PatternNode, subject_type: &ExprType) {
    match &mut pattern.kind {
      PatternKind::Underscore => {
        // matches anything and adds nothing to scope, so nothing to do here
      }

      PatternKind::Identifier(ident_node) => {
        self.add_value_binding(
          ident_node.name.clone(),
          subject_type.clone(),
          ident_node.pos,
        );
      }

      PatternKind::Literal(literal) => {
        let literal_type = self.analyze_literal(literal);

        if !literal_type.is_convertible_to(&subject_type) {
          self.error(
            pattern.pos,
            MismatchedTypes {
              expected: subject_type.clone(),
              actual: literal_type,
            },
          );
        }
      }

      PatternKind::Tuple(entry_patterns) => match subject_type {
        ExprType::Tuple(entry_types) => {
          if entry_patterns.len() != entry_types.len() {
            self.error(
              pattern.pos,
              PatternMismatchTupleSize {
                pattern_size: entry_patterns.len(),
                subject_size: entry_types.len(),
              },
            );
          }

          for i in 0..entry_patterns.len() {
            let entry_pattern = entry_patterns.get_mut(i).unwrap();
            let entry_type = entry_types.get(i).unwrap();

            self.destructure_pattern(entry_pattern, entry_type);
          }
        }

        _ => self.error(
          pattern.pos,
          PatternMismatchExpectedTuple {
            actual: subject_type.clone(),
          },
        ),
      },

      _ => {}
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
        definition.name.pos,
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
            ident.pos,
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
          self.add_value_binding(ident.name.clone(), param_type.clone(), ident.pos);
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
        self.add_value_binding(name.name.clone(), binding_type, name.pos);

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
  fn generate_constraints(&mut self, module: &mut ModuleNode) {
    for definition in &mut module.body {
      self.constraints_from_definition(definition)
    }
  }

  fn constraint(&mut self, type_a: ExprType, type_b: ExprType) {
    self.constraints.insert((type_a, type_b));
  }

  fn constraints_from_definition(&mut self, definition: &mut DefinitionNode) {
    match &mut definition.kind {
      DefinitionKind::Expr(expr) => {
        self.constraints_from_expr(expr);
        self.constraint(definition.inferred_type.clone(), expr.inferred_type.clone());
      }
      _ => {
        // todo :---)
      }
    }
  }

  fn constraints_from_expr(&mut self, expr: &mut ExprNode) {
    let inferred_type = expr.inferred_type.clone();

    match &mut expr.kind {
      ExprKind::Identifier(..) => { /* no constraints to add */ }

      ExprKind::Literal(literal) => self.constraints_from_literal(inferred_type, literal),

      ExprKind::Regex(..) => self.constraint(inferred_type, ExprType::Regex),

      ExprKind::Grouping(inner) => {
        self.constraints_from_expr(inner);
        self.constraint(inferred_type, inner.inferred_type.clone());
      }

      ExprKind::BinaryOperation { left, right, op } => {
        self.constraints_from_expr(left);
        self.constraints_from_expr(right);

        match op.kind {
          Operator::Addition => {
            // todo: floats?
            self.constraint(left.inferred_type.clone(), ExprType::Int);
            self.constraint(right.inferred_type.clone(), ExprType::Int);
            self.constraint(inferred_type.clone(), ExprType::Int);
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
          self.constraints_from_expr(expr);
          return_type = expr.inferred_type.clone();
        }

        // we know that this lambda must be a function that takes
        // the param types and returns the return type
        self.constraint(
          inferred_type,
          ExprType::Func(param_types, Box::new(return_type)),
        )
      }

      ExprKind::Call(CallNode { callee, args, .. }) => {
        let arg_types = args.iter().map(|a| a.inferred_type.clone()).collect();

        self.constraints_from_expr(callee);

        for arg in args {
          self.constraints_from_expr(arg);
        }

        // we know that the callee should be a function that takes
        // the given arg types and returns the type of this whole expr
        self.constraint(
          callee.inferred_type.clone(),
          ExprType::Func(arg_types, Box::new(inferred_type)),
        )
      }

      ExprKind::Let(LetNode { value, .. }) => {
        self.constraints_from_expr(value);

        // let expressions always evaluate to ()
        self.constraint(inferred_type, ExprType::Nothing)
      }

      _ => {
        // todo :---)
      }
    }
  }

  fn constraints_from_literal(&mut self, typ: ExprType, literal: &mut LiteralNode) {
    match &mut literal.kind {
      LiteralKind::Str(..) => self.constraint(typ, ExprType::String),
      LiteralKind::FloatDecimal(..) => self.constraint(typ, ExprType::Float),
      LiteralKind::IntDecimal(..)
      | LiteralKind::IntHex(..)
      | LiteralKind::IntBinary(..)
      | LiteralKind::IntOctal(..) => self.constraint(typ, ExprType::Int),
    }
  }
}

// Analysis methods
impl<'compiler> Analyzer<'compiler> {
  fn analyze_definition(&mut self, definition: &mut DefinitionNode) {
    let name = definition.name.name.clone();

    match &mut definition.kind {
      DefinitionKind::Expr(expr) => {
        let resolved_type = self.analyze_expr(expr);

        if let ExprType::Unknown = resolved_type {
          self.error(
            definition.name.pos,
            CouldNotInferDefinitionType { name: name.clone() },
          );
        }

        self.add_value_binding(name, resolved_type, definition.name.pos)
      }

      DefinitionKind::Alias(type_expr) => {
        let aliased_type = self.analyze_type_expr(type_expr);

        self.add_type_binding(name.clone(), aliased_type.clone(), definition.name.pos);

        self.add_value_binding(
          name.clone(),
          ExprType::Func(vec![aliased_type.clone()], Box::new(ExprType::Named(name))),
          definition.name.pos,
        );
      }
    };
  }

  fn analyze_type_expr(&mut self, type_expr: &mut TypeExprNode) -> ExprType {
    type_expr.to_type()
  }

  fn analyze_expr(&mut self, expr: &mut ExprNode) -> ExprType {
    match &mut expr.kind {
      ExprKind::Identifier(ident) => self.analyze_identifier(ident),
      ExprKind::Literal(literal) => self.analyze_literal(literal),
      ExprKind::Tuple(entries) => self.analyze_tuple_entries(entries),
      ExprKind::Record(entries) => self.analyze_record_entries(entries),
      ExprKind::EmptyTuple => ExprType::Nothing,
      ExprKind::Lambda(lambda) => self.analyze_lambda(lambda),
      ExprKind::Let(let_node) => self.analyze_let(let_node),
      ExprKind::Interpolation(parts) => self.analyze_interpolation(parts),
      ExprKind::Regex(..) => ExprType::Regex,
      ExprKind::Grouping(inner) => self.analyze_expr(inner),
      ExprKind::BinaryOperation { op, left, right } => self.analyze_binary_op(op, left, right),
      ExprKind::Call(call) => self.analyze_call(call),
      ExprKind::When(when) => self.analyze_when(when),
      ExprKind::If(if_node) => self.analyze_if(if_node),
      // TODO! more here!
      _ => ExprType::Unknown,
    }
  }

  fn analyze_when(&mut self, when: &mut WhenNode) -> ExprType {
    let subject_type = self.analyze_expr(&mut when.subject);

    let mut case_type = None;

    for case in &mut when.cases {
      self.enter_scope();

      self.destructure_pattern(&mut case.pattern, &subject_type);

      for expr in &mut case.body {
        let expr_type = self.analyze_expr(expr);

        if let Some(case_type) = &case_type {
          if !expr_type.is_convertible_to(case_type) {
            self.error(
              expr.pos,
              MismatchedTypesForWhenCases {
                expected: case_type.clone(),
                actual: expr_type,
              },
            )
          }
        } else {
          case_type = Some(expr_type);
        }
      }

      self.leave_scope();
    }

    case_type.expect("should have at least one case")
  }

  fn analyze_if(&mut self, if_node: &mut IfNode) -> ExprType {
    let subject_type = self.analyze_expr(&mut if_node.subject);

    self.destructure_pattern(&mut if_node.pattern, &subject_type);

    for expr in &mut if_node.body {
      self.analyze_expr(expr);
    }

    // ifs always have type nothing
    ExprType::Nothing
  }

  fn analyze_call(&mut self, call: &mut CallNode) -> ExprType {
    let callee_type = self.analyze_expr(&mut call.callee);

    if let ExprType::Func(param_types, return_type) = callee_type {
      let arg_types: Vec<ExprType> = call
        .args
        .iter_mut()
        .map(|arg| self.analyze_expr(arg))
        .collect();

      if arg_types.len() != param_types.len() {
        self.error(
          call.pos,
          IncorrectNumberOfArguments {
            arg_types,
            param_types,
          },
        );
      } else {
        for i in 0..arg_types.len() {
          if !arg_types[i].is_convertible_to(&param_types[i]) {
            let arg_pos = call.args[i].pos;

            self.error(
              arg_pos,
              MismatchedTypes {
                expected: param_types[i].clone(),
                actual: arg_types[i].clone(),
              },
            )
          }
        }
      }

      // return the expected return type even if the args were incorrect
      // to give type analysis something to work with
      return *return_type.clone();
    } else {
      self.error(
        call.callee.pos,
        CalleeNotFunction {
          actual: callee_type,
        },
      )
    }

    ExprType::Unknown
  }

  fn analyze_binary_op(
    &mut self,
    op: &mut OperatorNode,
    left: &mut ExprNode,
    right: &mut ExprNode,
  ) -> ExprType {
    match op.kind {
      Operator::Addition
      | Operator::SubtractionOrNegation
      | Operator::Multiplication
      | Operator::Exponentiation
      | Operator::Division
      | Operator::Remainder => {
        let left_type = self.analyze_expr(left);
        let right_type = self.analyze_expr(right);

        match (&left_type, &right_type) {
          (ExprType::Int, ExprType::Int) => return ExprType::Int,
          (ExprType::Float, ExprType::Float) => return ExprType::Float,
          (ExprType::Int, _) | (_, ExprType::Int) => self.error(
            op.pos,
            MismatchedTypesForOperator {
              op: op.kind.clone(),
              expected: ExprType::Int,
              actual_left: left_type,
              actual_right: right_type,
            },
          ),
          (ExprType::Float, _) | (_, ExprType::Float) => self.error(
            op.pos,
            MismatchedTypesForOperator {
              op: op.kind.clone(),
              expected: ExprType::Float,
              actual_left: left_type,
              actual_right: right_type,
            },
          ),
          _ => self.error(
            op.pos,
            MismatchedTypesForOperator {
              op: op.kind.clone(),
              expected: ExprType::Int,
              actual_left: left_type,
              actual_right: right_type,
            },
          ),
        };

        ExprType::Unknown
      }

      Operator::LogicalAnd | Operator::LogicalOr => {
        let left_type = self.analyze_expr(left);
        let right_type = self.analyze_expr(right);

        match (&left_type, &right_type) {
          (ExprType::Bool, ExprType::Bool) => return ExprType::Bool,
          _ => self.error(
            op.pos,
            MismatchedTypesForOperator {
              op: op.kind.clone(),
              expected: ExprType::Bool,
              actual_left: left_type,
              actual_right: right_type,
            },
          ),
        };

        ExprType::Unknown
      }

      Operator::Equality | Operator::IndexAccess => {
        let left_type = self.analyze_expr(left);
        let right_type = self.analyze_expr(right);

        if left_type != right_type {
          self.error(
            op.pos,
            MismatchedTypesForOperator {
              op: op.kind.clone(),
              expected: left_type.clone(),
              actual_left: left_type,
              actual_right: right_type,
            },
          );

          return ExprType::Unknown;
        }

        left_type
      }

      Operator::FieldAccess => {
        let receiver_type = self.analyze_expr(left);

        // The parser allows any expression on the right of the field
        // access operator, but we want to limit it to decimal literals
        // or identifiers as field names.
        let field_name = match &right.kind {
          ExprKind::Literal(LiteralNode {
            kind: LiteralKind::IntDecimal(value),
            ..
          }) => {
            format!("{}", value)
          }
          ExprKind::Identifier(IdentifierNode { name, .. }) => name.clone(),
          _ => {
            self.error(right.pos, InvalidFieldAccess);
            return ExprType::Unknown;
          }
        };

        match self.get_field_type(&receiver_type, &field_name) {
          Some(field_type) => field_type,
          None => {
            self.error(
              right.pos,
              UndefinedFieldForType {
                field_name: field_name.clone(),
                receiver_type,
              },
            );

            ExprType::Unknown
          }
        }
      }

      // TODO: more binary ops!
      _ => ExprType::Unknown,
    }
  }

  fn analyze_lambda(&mut self, lambda: &mut LambdaNode) -> ExprType {
    let mut param_types = Vec::new();
    let mut return_type = ExprType::Unknown;

    self.enter_scope();

    for LambdaParamNode { ident, .. } in &lambda.params {
      let name = ident.name.clone();

      self.add_value_binding(name, ExprType::Unknown, ident.pos);

      param_types.push(ExprType::Unknown);
    }

    for expr in &mut lambda.body {
      return_type = self.analyze_expr(expr);
    }

    self.leave_scope();

    if param_types.is_empty() {
      param_types.push(ExprType::Nothing);
    }

    ExprType::Func(param_types, Box::new(return_type))
  }

  fn analyze_identifier(&mut self, ident: &mut IdentifierNode) -> ExprType {
    if let Some(binding) = self.get_value_binding(&ident.name) {
      binding.typ.clone()
    } else {
      self.error(
        ident.pos,
        NameNotBound {
          name: ident.name.clone(),
        },
      );

      ExprType::Unknown
    }
  }

  fn analyze_let(&mut self, let_node: &mut LetNode) -> ExprType {
    let name = let_node.name.name.clone();
    let expr_type = self.analyze_expr(&mut let_node.value);

    self.add_value_binding(name, expr_type.clone(), let_node.name.pos);

    expr_type
  }

  fn analyze_interpolation(&mut self, parts: &mut Vec<ExprNode>) -> ExprType {
    for part in parts {
      match self.analyze_expr(part) {
        ExprType::String => {}
        other_type => self.error(
          part.pos,
          MismatchedTypes {
            expected: ExprType::String,
            actual: other_type,
          },
        ),
      }
    }

    ExprType::String
  }

  fn analyze_tuple_entries(&mut self, entries: &mut Vec<ExprNode>) -> ExprType {
    let mut entry_types = Vec::new();

    for entry in entries {
      let entry_type = self.analyze_expr(entry);

      entry_types.push(entry_type);
    }

    ExprType::Tuple(entry_types)
  }

  fn analyze_record_entries(&mut self, entries: &mut Vec<(IdentifierNode, ExprNode)>) -> ExprType {
    let mut entry_types = Vec::new();

    for (label, entry) in entries {
      let entry_type = self.analyze_expr(entry);

      entry_types.push((label.name.clone(), entry_type));
    }

    ExprType::Record(entry_types)
  }

  fn analyze_literal(&mut self, literal: &mut LiteralNode) -> ExprType {
    match &mut literal.kind {
      LiteralKind::IntDecimal(..) => ExprType::Int,
      LiteralKind::IntBinary(..) => ExprType::Int,
      LiteralKind::IntOctal(..) => ExprType::Int,
      LiteralKind::IntHex(..) => ExprType::Int,
      LiteralKind::FloatDecimal(..) => ExprType::Float,
      LiteralKind::Str(..) => ExprType::String,
    }
  }
}
