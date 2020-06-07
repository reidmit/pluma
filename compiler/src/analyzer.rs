use crate::analysis_error::{AnalysisError, AnalysisErrorKind};
use crate::diagnostics::Diagnostic;
use crate::scope::{BindingKind, Scope, TypeBindingKind};
use crate::visitor::Visitor;
use pluma_ast::nodes::*;
use pluma_ast::value_type::ValueType;

pub struct Analyzer<'a> {
  pub diagnostics: Vec<Diagnostic>,
  scope: &'a mut Scope,
}

impl<'a> Analyzer<'a> {
  pub fn new(scope: &'a mut Scope) -> Analyzer<'a> {
    Analyzer {
      scope,
      diagnostics: Vec::new(),
    }
  }

  fn error(&mut self, err: AnalysisError) {
    let pos = err.pos;
    self.diagnostic(Diagnostic::error(err).with_pos(pos))
  }

  fn diagnostic(&mut self, diag: Diagnostic) {
    self.diagnostics.push(diag)
  }

  fn destructure_pattern(&mut self, pattern: &PatternNode, typ: &ValueType) {
    match &pattern.kind {
      PatternKind::Identifier(ident_node) => {
        let existing_binding = self.scope.get_binding(&ident_node.name);

        if existing_binding.is_some() {
          self.error(AnalysisError {
            pos: ident_node.pos,
            kind: AnalysisErrorKind::NameAlreadyInScope(ident_node.name.clone()),
          })
        } else {
          self.scope.add_binding(
            BindingKind::Let,
            ident_node.name.clone(),
            typ.clone(),
            ident_node.pos,
          );
        }
      }

      PatternKind::Tuple(element_patterns) => match typ {
        ValueType::Tuple(element_types) => {
          if element_patterns.len() != element_types.len() {
            return self.error(AnalysisError {
              pos: pattern.pos,
              kind: AnalysisErrorKind::PatternMismatchTupleSize {
                pattern_size: element_patterns.len(),
                value_size: element_types.len(),
              },
            });
          }

          for i in 0..element_patterns.len() {
            let element_pattern = element_patterns.get(i).unwrap();
            let element_type = element_types.get(i).unwrap();

            self.destructure_pattern(&element_pattern, &element_type);
          }
        }

        typ => self.error(AnalysisError {
          pos: pattern.pos,
          kind: AnalysisErrorKind::PatternMismatchExpectedTuple(typ.clone()),
        }),
      },

      PatternKind::Constructor(ident, param_pattern) => {
        let binding = self.scope.get_binding(&ident.name);

        if binding.is_none() {
          self.error(AnalysisError {
            pos: pattern.pos,
            kind: AnalysisErrorKind::UndefinedTypeConstructor(ident.name.clone()),
          });

          return;
        }

        let existing_binding = binding.unwrap();

        // Constructor patterns are only allowed for struct types
        if existing_binding.kind != BindingKind::StructConstructor {
          return;
        }

        let (param_type, constructor_type) = match &existing_binding.typ {
          ValueType::Func(param_types, return_type) => {
            (param_types.get(0).unwrap().clone(), (**return_type).clone())
          }
          _ => return,
        };

        let actual_type = typ.clone();

        if constructor_type != actual_type {
          self.error(AnalysisError {
            pos: pattern.pos,
            kind: AnalysisErrorKind::PatternMismatchExpectedConstructor {
              constructor_type,
              actual_type,
            },
          });

          return;
        }

        self.destructure_pattern(param_pattern, &param_type);
      }

      PatternKind::Underscore => {}

      PatternKind::Literal(..) | PatternKind::Interpolation(..) => self.error(AnalysisError {
        pos: pattern.pos,
        kind: AnalysisErrorKind::CannotAssignToLiteral,
      }),
    }
  }
}

impl<'a> Visitor for Analyzer<'a> {
  fn leave_module(&mut self, _node: &mut ModuleNode) {
    println!("end scope: {:#?}", self.scope);
  }

  fn leave_call(&mut self, node: &mut CallNode) {
    let callee_type = &node.callee.typ;

    match callee_type {
      ValueType::Func(param_types, return_type) => {
        if param_types.len() != node.args.len() {
          self.error(AnalysisError {
            pos: node.pos,
            kind: AnalysisErrorKind::IncorrectNumberOfArguments {
              expected: param_types.len(),
              actual: node.args.len(),
            },
          })
        }

        for i in 0..param_types.len() {
          let param_type = param_types.get(i).unwrap();
          let given_type = &node.args.get(i).unwrap().typ;

          if param_type != given_type {
            let pos = node.args.get(i).unwrap().pos;

            self.error(AnalysisError {
              pos,
              kind: AnalysisErrorKind::ParameterTypeMismatch {
                expected: param_type.clone(),
                actual: given_type.clone(),
              },
            })
          }
        }

        node.typ = *return_type.clone();
      }

      _ => self.error(AnalysisError {
        pos: node.pos,
        kind: AnalysisErrorKind::CalleeNotCallable(callee_type.clone()),
      }),
    }
  }

  fn leave_let(&mut self, node: &mut LetNode) {
    self.destructure_pattern(&mut node.pattern, &mut node.value.typ)
  }

  fn enter_expr(&mut self, node: &mut ExprNode) {
    match &node.kind {
      ExprKind::Block { params, .. } => {
        self.scope.enter();

        for param in params {
          self.scope.add_binding(
            BindingKind::Param,
            param.name.clone(),
            ValueType::Unknown,
            param.pos,
          );
        }
      }

      _ => {}
    }
  }

  fn leave_expr(&mut self, node: &mut ExprNode) {
    match &node.kind {
      ExprKind::Identifier(ident_node) => {
        match self.scope.get_binding(&ident_node.name) {
          Some(binding) => node.typ = binding.typ.clone(),
          None => self.error(AnalysisError {
            pos: node.pos,
            kind: AnalysisErrorKind::UndefinedName(ident_node.name.clone()),
          }),
        };
      }

      ExprKind::MultiPartIdentifier(ident_nodes) => {
        let names = ident_nodes
          .iter()
          .map(|node| node.name.clone())
          .collect::<Vec<String>>();

        let merged_name = names.join(" ");

        match self.scope.get_binding(&merged_name) {
          Some(binding) => node.typ = binding.typ.clone(),
          None => self.error(AnalysisError {
            pos: node.pos,
            kind: AnalysisErrorKind::UndefinedMultiPartName(names),
          }),
        };
      }

      ExprKind::Literal(lit_node) => match &lit_node.kind {
        LiteralKind::IntDecimal { .. } => node.typ = ValueType::Named("Int".to_owned()),
        LiteralKind::IntBinary { .. } => node.typ = ValueType::Named("Int".to_owned()),
        LiteralKind::IntHex { .. } => node.typ = ValueType::Named("Int".to_owned()),
        LiteralKind::IntOctal { .. } => node.typ = ValueType::Named("Int".to_owned()),
        LiteralKind::FloatDecimal { .. } => node.typ = ValueType::Named("Float".to_owned()),
        LiteralKind::Str { .. } => node.typ = ValueType::Named("String".to_owned()),
      },

      ExprKind::Call(call_node) => node.typ = call_node.typ.clone(),

      ExprKind::Assignment { left, right } => {
        let existing_binding = self.scope.get_binding(&left.name);

        if let Some(binding) = existing_binding {
          let current_type = binding.typ.clone();
          let new_type = right.typ.clone();

          if current_type != new_type {
            self.error(AnalysisError {
              pos: right.pos,
              kind: AnalysisErrorKind::ReassignmentTypeMismatch {
                expected: current_type,
                actual: new_type,
              },
            })
          }
        }
      }

      ExprKind::Block { params, body } => {
        let mut param_types = Vec::new();
        let mut return_type = ValueType::Nothing;

        for _param in params {
          param_types.push(ValueType::Unknown);
        }

        for stmt in body {
          if let StatementKind::Expr(expr) = &stmt.kind {
            return_type = expr.typ.clone();
          }
        }

        node.typ = ValueType::Func(param_types, Box::new(return_type));

        if let Err(diagnostics) = self.scope.exit() {
          for diagnostic in diagnostics {
            self.diagnostic(diagnostic);
          }
        }
      }

      ExprKind::Tuple(elements) => {
        let mut element_types = Vec::new();

        for element in elements {
          element_types.push(element.typ.clone());
        }

        node.typ = ValueType::Tuple(element_types);
      }

      ExprKind::Interpolation(parts) => {
        let string_type = ValueType::Named("String".to_owned());

        for part in parts {
          if part.typ != string_type {
            self.error(AnalysisError {
              pos: part.pos,
              kind: AnalysisErrorKind::TypeMismatchInStringInterpolation(part.typ.clone()),
            })
          }
        }

        node.typ = string_type;
      }

      ExprKind::FieldAccess { receiver, field } => {
        let receiver_type_binding = self.scope.get_type_binding(&receiver.typ).unwrap();

        if let TypeBindingKind::Struct { fields } = &receiver_type_binding.kind {
          match fields.get(&field.name) {
            Some(binding) => node.typ = binding.typ.clone(),

            None => self.error(AnalysisError {
              pos: field.pos,
              kind: AnalysisErrorKind::UndefinedFieldForType {
                field_name: field.name.clone(),
                receiver_type: receiver.typ.clone(),
              },
            }),
          }
        } else {
          self.error(AnalysisError {
            pos: field.pos,
            kind: AnalysisErrorKind::UndefinedFieldForType {
              field_name: field.name.clone(),
              receiver_type: receiver.typ.clone(),
            },
          })
        }
      }

      ExprKind::MethodAccess {
        receiver,
        method_parts,
      } => {
        let receiver_type_binding = match self.scope.get_type_binding(&receiver.typ) {
          Some(binding) => binding,
          _ => return,
        };

        let method_name_parts = method_parts
          .iter()
          .map(|n| n.name.clone())
          .collect::<Vec<String>>();

        if let Some(method_type) = receiver_type_binding.methods.get(&method_name_parts) {
          node.typ = method_type.clone();
        } else {
          let pos = (
            method_parts.first().unwrap().pos.0,
            method_parts.last().unwrap().pos.1,
          );

          self.error(AnalysisError {
            pos,
            kind: AnalysisErrorKind::UndefinedMethodForType {
              method_name_parts,
              receiver_type: receiver.typ.clone(),
            },
          })
        }
      }

      ExprKind::TypeAssertion {
        expr,
        asserted_type,
      } => {
        let expr_type = &expr.typ;
        let asserted_type = &asserted_type.typ;

        if expr_type != asserted_type {
          self.error(AnalysisError {
            pos: node.pos,
            kind: AnalysisErrorKind::TypeMismatchInTypeAssertion {
              expected: asserted_type.clone(),
              actual: expr_type.clone(),
            },
          });

          return;
        }

        node.typ = asserted_type.clone();
      }

      ExprKind::EmptyTuple => node.typ = ValueType::Nothing,

      t => todo!("more expr kinds: {:#?}", t),
    }
  }
}
