use crate::analysis_error::{AnalysisError, AnalysisErrorKind};
use crate::scope::{Binding, BindingKind, Scope, TypeBindingKind};
use crate::type_utils;
use pluma_ast::*;
use pluma_diagnostics::*;
use pluma_visitor::*;
use std::collections::HashMap;
use std::iter::Iterator;

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

  fn check_result(&mut self, result: Result<(), Diagnostic>) {
    if let Err(diag) = result {
      self.diagnostic(diag);
    }
  }

  fn check_results(&mut self, result: Result<(), Vec<Diagnostic>>) {
    if let Err(diags) = result {
      for diag in diags {
        self.diagnostic(diag);
      }
    }
  }

  fn collect_def(
    &mut self,
    pos: Position,
    generic_type_constraints: &mut GenericTypeConstraints,
    kind: &mut DefKind,
    return_type: &Option<TypeExprNode>,
  ) {
    let mut constraints_map = HashMap::new();

    for (constraint_name, constraint_type_id) in generic_type_constraints {
      let constraint = if constraint_type_id.generics.is_empty() {
        TypeConstraint::NamedTrait(constraint_type_id.name.clone())
      } else {
        TypeConstraint::GenericTrait(
          constraint_type_id.name.clone(),
          constraint_type_id
            .generics
            .iter()
            .map(|type_expr| type_utils::type_expr_to_value_type(type_expr))
            .collect(),
        )
      };

      constraints_map.insert(
        constraint_name.name.clone(),
        ValueType::Constrained(constraint),
      );
    }

    match kind {
      DefKind::Function { signature } => {
        let mut name_parts = Vec::new();
        let mut param_types = Vec::new();

        for (part_name, part_type) in signature {
          name_parts.push(part_name.name.clone());

          let param_type = type_utils::type_expr_to_value_type(part_type);

          if let ValueType::Named(name) = &param_type {
            if let Some(constraint) = constraints_map.get(name) {
              param_types.push(constraint.clone());
            } else {
              param_types.push(param_type);
            }
          } else {
            param_types.push(param_type);
          }
        }

        let return_type = match return_type {
          Some(ret) => type_utils::type_expr_to_value_type(&ret),
          None => ValueType::Nothing,
        };

        let def_type = ValueType::Func(param_types, Box::new(return_type));
        let merged_name = name_parts.join(" ");

        self
          .scope
          .add_binding(BindingKind::Def, merged_name, def_type, pos);
      }

      DefKind::Method {
        receiver,
        signature,
      } => {
        let receiver_type = ValueType::Named(receiver.name.clone());

        let return_type = match return_type {
          Some(type_expr) => type_utils::type_expr_to_value_type(&type_expr),
          None => ValueType::Nothing,
        };

        let mut method_parts = Vec::new();
        let mut param_types = Vec::new();

        for (part_name, part_type_expr) in signature {
          method_parts.push(part_name.name.clone());
          param_types.push(type_utils::type_expr_to_value_type(part_type_expr));
        }

        let result = self.scope.add_type_method(
          receiver_type,
          method_parts,
          param_types,
          return_type,
          receiver.pos,
        );

        self.check_result(result);
      }

      DefKind::BinaryOperator { op, left, right } => {
        let receiver_type = type_utils::type_ident_to_value_type(left);

        let param_type = type_utils::type_ident_to_value_type(right);

        let return_type = match return_type {
          Some(type_expr) => type_utils::type_expr_to_value_type(&type_expr),
          None => ValueType::Nothing,
        };

        let method_parts = vec!["$".to_owned(), op.name.clone(), "$".to_owned()];
        let param_types = vec![param_type];

        let result = self.scope.add_type_method(
          receiver_type,
          method_parts,
          param_types,
          return_type,
          left.pos,
        );

        self.check_result(result);
      }

      DefKind::UnaryOperator { op, right } => {
        let receiver_type = type_utils::type_ident_to_value_type(right);

        let return_type = match return_type {
          Some(type_expr) => type_utils::type_expr_to_value_type(&type_expr),
          None => ValueType::Nothing,
        };

        let method_parts = vec![op.name.clone(), "$".to_owned()];
        let param_types = vec![];

        let result = self.scope.add_type_method(
          receiver_type,
          method_parts,
          param_types,
          return_type,
          right.pos,
        );

        self.check_result(result);
      }
    }
  }

  fn collect_const(&mut self, node: &mut ConstNode) {
    let const_type = match &node.value.kind {
      ExprKind::Literal(lit) => self.analyze_literal(lit),
      _ => {
        self.error(AnalysisError {
          pos: node.value.pos,
          kind: AnalysisErrorKind::InvalidValueForConst,
        });

        return;
      }
    };

    self.scope.add_binding(
      BindingKind::Const,
      node.name.name.clone(),
      const_type,
      node.pos,
    );
  }

  fn collect_type_def(&mut self, node: &mut TypeDefNode) {
    let typ = ValueType::Named(node.name.name.clone());

    match &node.kind {
      TypeDefKind::Enum { variants } => {
        self
          .scope
          .add_type_binding(typ.clone(), TypeBindingKind::Enum, node.name.pos);

        for variant in variants {
          match &variant.kind {
            EnumVariantKind::Identifier(ident_node) => {
              let variant_name = ident_node.name.clone();
              let variant_type = typ.clone();

              self.scope.add_binding(
                BindingKind::EnumVariant,
                variant_name,
                variant_type,
                ident_node.pos,
              );
            }

            EnumVariantKind::Constructor(constructor_node, param_node) => {
              let constructor_name = constructor_node.name.clone();
              let param_type = type_utils::type_expr_to_value_type(param_node);
              let constructor_type = ValueType::Func(vec![param_type], Box::new(typ.clone()));

              self.scope.add_binding(
                BindingKind::EnumVariant,
                constructor_name,
                constructor_type,
                variant.pos,
              );
            }
          }
        }
      }

      TypeDefKind::Struct { fields } => {
        let mut param_types = Vec::new();
        let mut fields_map = HashMap::new();

        for field in fields {
          let (field_id, field_type) = field;
          param_types.push(type_utils::type_expr_to_value_type(field_type));

          fields_map.insert(
            field_id.name.clone(),
            Binding {
              kind: BindingKind::Field,
              ref_count: 0,
              pos: field_id.pos,
              typ: type_utils::type_expr_to_value_type(field_type),
            },
          );
        }

        self.scope.add_type_binding(
          typ.clone(),
          TypeBindingKind::Struct { fields: fields_map },
          node.name.pos,
        );

        let param_tuple_type = ValueType::UnlabeledTuple(param_types);
        let constructor_type = ValueType::Func(vec![param_tuple_type], Box::new(typ));

        self.scope.add_binding(
          BindingKind::StructConstructor,
          node.name.name.clone(),
          constructor_type,
          node.name.pos,
        );
      }

      TypeDefKind::Alias { .. } => {
        self
          .scope
          .add_type_binding(typ.clone(), TypeBindingKind::Alias, node.name.pos);
      }

      TypeDefKind::Trait { fields, .. } => {
        let mut fields_map = HashMap::new();

        for field in fields {
          let (field_id, field_type) = field;

          fields_map.insert(
            field_id.name.clone(),
            Binding {
              kind: BindingKind::Field,
              ref_count: 0,
              pos: field_id.pos,
              typ: type_utils::type_expr_to_value_type(field_type),
            },
          );
        }

        self.scope.add_type_binding(
          typ.clone(),
          TypeBindingKind::Trait { fields: fields_map },
          node.name.pos,
        );
      }
    }
  }

  fn collect_intrinsic_type_def(&mut self, node: &mut IntrinsicTypeDefNode) {
    let intrinsic_type = match &node.name.name[..] {
      "Int" => Some(ValueType::Int),
      "Float" => Some(ValueType::Float),
      "String" => Some(ValueType::String),
      _ => None,
    };

    if let Some(typ) = intrinsic_type {
      self
        .scope
        .add_type_binding(typ, TypeBindingKind::IntrinsicType, node.name.pos);
    }
  }

  fn destructure_pattern(&mut self, pattern: &PatternNode, typ: &ValueType) {
    match &pattern.kind {
      PatternKind::Identifier(ident_node, _is_mutable) => {
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

      PatternKind::UnlabeledTuple(element_patterns) => match typ {
        ValueType::UnlabeledTuple(element_types) => {
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

      PatternKind::LabeledTuple(element_patterns) => match typ {
        ValueType::LabeledTuple(element_types) => {
          if element_patterns.len() != element_types.len() {
            return self.error(AnalysisError {
              pos: pattern.pos,
              kind: AnalysisErrorKind::PatternMismatchTupleSize {
                pattern_size: element_patterns.len(),
                value_size: element_types.len(),
              },
            });
          }

          for (label, element_pattern) in element_patterns {
            let matching_type_field = element_types
              .iter()
              .find(|(field_label, _)| *field_label == label.name);

            match matching_type_field {
              None => self.error(AnalysisError {
                pos: label.pos,
                kind: AnalysisErrorKind::PatternMismatchUnknownField {
                  field_name: label.name.clone(),
                  value_type: typ.clone(),
                },
              }),
              Some((_, element_type)) => {
                self.destructure_pattern(&element_pattern, &element_type);
              }
            }
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

  fn analyze_block(&mut self, node: &mut BlockNode) -> ValueType {
    let mut param_types = Vec::new();
    let mut return_type = ValueType::Nothing;

    if node.params.is_empty() {
      param_types.push(ValueType::Nothing);
    } else {
      for _param in &node.params {
        param_types.push(ValueType::Unknown);
      }
    }

    for stmt in &node.body {
      if let StatementKind::Expr(expr) = &stmt.kind {
        return_type = expr.typ.clone();
      }
    }

    ValueType::Func(param_types, Box::new(return_type))
  }

  fn analyze_call(&mut self, node: &mut CallNode) -> ValueType {
    self.analyze_expr(&mut node.callee);

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
          let arg = node.args.get_mut(i).unwrap();
          self.analyze_expr(arg);

          let param_type = param_types.get(i).unwrap();
          let given_type = &arg.typ;

          if param_type != given_type {
            let pos = arg.pos;

            self.error(AnalysisError {
              pos,
              kind: AnalysisErrorKind::ParameterTypeMismatch {
                expected: param_type.clone(),
                actual: given_type.clone(),
              },
            })
          }
        }

        *return_type.clone()
      }

      _ => {
        self.error(AnalysisError {
          pos: node.pos,
          kind: AnalysisErrorKind::CalleeNotCallable(callee_type.clone()),
        });

        ValueType::Unknown
      }
    }
  }

  fn analyze_def(&mut self, node: &mut DefNode) {
    self.scope.enter();

    match &mut node.kind {
      DefKind::Function { signature } => {
        let params = &node.block.params;

        if params.len() != signature.len() {
          let start = params.first().map(|p| p.pos.0).unwrap_or(node.pos.0);
          let end = params.last().map(|p| p.pos.1).unwrap_or(node.pos.1);

          self.error(AnalysisError {
            pos: (start, end),
            kind: AnalysisErrorKind::ParamCountMismatchInDefinition {
              expected: signature.len(),
              actual: params.len(),
            },
          })
        }

        for (_part_name, part_type) in signature {
          self.analyze_type_expr(part_type);
        }
      }

      DefKind::Method {
        receiver,
        signature,
      } => {
        self.analyze_type_identifier(receiver);

        for (_part_name, part_type) in signature {
          self.analyze_type_expr(part_type);
        }
      }

      DefKind::BinaryOperator { left, right, .. } => {
        self.analyze_type_identifier(left);
        self.analyze_type_identifier(right);
      }

      DefKind::UnaryOperator { right, .. } => {
        self.analyze_type_identifier(right);
      }
    }

    if let Some(return_type) = &mut node.return_type {
      self.analyze_type_expr(return_type);
    }

    let results = self.scope.exit();

    self.check_results(results);
  }

  fn analyze_expr(&mut self, node: &mut ExprNode) {
    match &mut node.kind {
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

      ExprKind::BinaryOperation { op, left, right } => {
        self.analyze_expr(left);
        self.analyze_expr(right);

        let receiver_type_binding = match self.scope.get_type_binding(&left.typ) {
          Some(binding) => binding,
          _ => return,
        };

        let method_name_parts = vec!["$".to_owned(), op.name.clone(), "$".to_owned()];

        if let Some(method_type) = receiver_type_binding.methods.get(&method_name_parts) {
          let param_types = method_type.func_param_types();
          let first_param_type = param_types.first().unwrap();

          node.typ = method_type.func_return_type();

          if right.typ != *first_param_type {
            self.error(AnalysisError {
              pos: right.pos,
              kind: AnalysisErrorKind::ParameterTypeMismatch {
                expected: first_param_type.clone(),
                actual: right.typ.clone(),
              },
            })
          }
        } else {
          self.error(AnalysisError {
            pos: op.pos,
            kind: AnalysisErrorKind::UndefinedBinaryOperatorForType {
              op_name: op.name.clone(),
              receiver_type: left.typ.clone(),
              param_type: right.typ.clone(),
            },
          })
        }
      }

      ExprKind::Block(block) => node.typ = self.analyze_block(block),

      ExprKind::Call(call_node) => node.typ = self.analyze_call(call_node),

      ExprKind::EmptyTuple => node.typ = ValueType::Nothing,

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

      ExprKind::Grouping(inner) => {
        self.analyze_expr(inner);
        node.typ = inner.typ.clone();
      }

      ExprKind::Identifier(ident_node) => {
        node.typ = self.analyze_identifier(ident_node);
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

      ExprKind::Literal(lit_node) => node.typ = self.analyze_literal(lit_node),

      ExprKind::MethodAccess {
        receiver,
        method_parts,
      } => {
        self.analyze_expr(receiver);

        let receiver_type_binding = match self.scope.get_type_binding(&receiver.typ) {
          Some(binding) => binding,
          _ => return,
        };

        // There is a special case here, where if we are calling a function that's a field
        // on a struct, rather than a method, it will be parsed as a MethodAccess at this point.
        // Check to see if we're in that case:
        if let TypeBindingKind::Struct { fields } = &receiver_type_binding.kind {
          if method_parts.len() == 1 {
            let potential_field_name = &method_parts[0].name.clone();

            if let Some(field_binding) = fields.get(potential_field_name) {
              node.typ = field_binding.typ.clone();
              return;
            }
          }
        }

        // If we didn't get into that special case, carry on and analyze this as a normal
        // method call.
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

      ExprKind::Match(match_node) => {
        let _subject_type = &match_node.subject.typ;
        let mut case_type: Option<ValueType> = None;

        for case in &match_node.cases {
          if let Some(expected_case_type) = &case_type {
            let actual_case_type = &case.body.typ;

            // TODO: more than equality comparison?
            if expected_case_type != actual_case_type {
              self.error(AnalysisError {
                pos: case.body.pos,
                kind: AnalysisErrorKind::TypeMismatchInMatchCase {
                  expected: expected_case_type.clone(),
                  actual: actual_case_type.clone(),
                },
              });
            }
          } else {
            case_type = Some(case.body.typ.clone());
          }
        }

        node.typ = case_type.unwrap().clone();
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

      ExprKind::TypeAssertion {
        expr,
        asserted_type,
      } => {
        self.analyze_type_expr(asserted_type);
        self.analyze_expr(expr);

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

      ExprKind::UnaryOperation { op, right } => {
        let receiver_type_binding = match self.scope.get_type_binding(&right.typ) {
          Some(binding) => binding,
          _ => return,
        };

        let method_name_parts = vec![op.name.clone(), "$".to_owned()];

        if let Some(method_type) = receiver_type_binding.methods.get(&method_name_parts) {
          node.typ = method_type.func_return_type();
        } else {
          self.error(AnalysisError {
            pos: op.pos,
            kind: AnalysisErrorKind::UndefinedUnaryOperatorForType {
              op_name: op.name.clone(),
              receiver_type: right.typ.clone(),
            },
          })
        }
      }

      ExprKind::UnlabeledTuple(entries) => {
        let mut entry_types = Vec::new();

        for entry in entries {
          self.analyze_expr(entry);
          entry_types.push(entry.typ.clone());
        }

        node.typ = ValueType::UnlabeledTuple(entry_types);
      }

      ExprKind::LabeledTuple(entries) => {
        let mut entry_types = Vec::new();

        for (label, entry) in entries {
          self.analyze_expr(entry);
          entry_types.push((label.name.clone(), entry.typ.clone()));
        }

        node.typ = ValueType::LabeledTuple(entry_types);
      }

      _other => todo!("more expr kinds!"),
    }
  }

  fn analyze_identifier(&mut self, node: &IdentifierNode) -> ValueType {
    match self.scope.get_binding(&node.name) {
      Some(binding) => binding.typ.clone(),
      None => {
        self.error(AnalysisError {
          pos: node.pos,
          kind: AnalysisErrorKind::UndefinedName(node.name.clone()),
        });

        ValueType::Unknown
      }
    }
  }

  fn analyze_intrinsic_def(&mut self, node: &mut IntrinsicDefNode) {
    match &mut node.kind {
      DefKind::Function { signature } => {
        for (_part_name, part_type) in signature {
          self.analyze_type_expr(part_type);
        }
      }

      DefKind::Method {
        receiver,
        signature,
      } => {
        self.analyze_type_identifier(receiver);

        for (_part_name, part_type) in signature {
          self.analyze_type_expr(part_type);
        }
      }

      DefKind::BinaryOperator { left, right, .. } => {
        self.analyze_type_identifier(left);
        self.analyze_type_identifier(right);
      }

      DefKind::UnaryOperator { right, .. } => {
        self.analyze_type_identifier(right);
      }
    }

    if let Some(return_type) = &mut node.return_type {
      self.analyze_type_expr(return_type);
    }
  }

  fn analyze_literal(&mut self, node: &LiteralNode) -> ValueType {
    match &node.kind {
      LiteralKind::IntDecimal { .. } => ValueType::Int,
      LiteralKind::IntBinary { .. } => ValueType::Int,
      LiteralKind::IntHex { .. } => ValueType::Int,
      LiteralKind::IntOctal { .. } => ValueType::Int,
      LiteralKind::FloatDecimal { .. } => ValueType::Float,
      LiteralKind::Str { .. } => ValueType::String,
    }
  }

  fn analyze_type_identifier(&mut self, node: &mut TypeIdentifierNode) -> ValueType {
    let named_value_type = type_utils::type_ident_to_value_type(&node);

    match self.scope.get_type_binding(&named_value_type) {
      Some(binding) => binding.ref_count += 1,
      None => self.error(AnalysisError {
        pos: node.pos,
        kind: AnalysisErrorKind::UndefinedType(named_value_type.clone()),
      }),
    }

    named_value_type
  }

  fn analyze_type_expr(&mut self, node: &mut TypeExprNode) {
    let typ = match &mut node.kind {
      TypeExprKind::EmptyTuple => ValueType::Nothing,

      TypeExprKind::Grouping(inner) => {
        self.analyze_type_expr(inner);
        inner.typ.clone()
      }

      TypeExprKind::Single(ident) => self.analyze_type_identifier(ident),

      TypeExprKind::UnlabeledTuple(entries) => {
        let mut entry_types = Vec::new();

        for entry in entries {
          self.analyze_type_expr(entry);
          entry_types.push(entry.typ.clone());
        }

        ValueType::UnlabeledTuple(entry_types)
      }

      TypeExprKind::LabeledTuple(entries) => {
        let mut entry_types = Vec::new();

        for (label_ident, entry) in entries {
          self.analyze_type_expr(entry);
          entry_types.push((label_ident.name.clone(), entry.typ.clone()));
        }

        ValueType::LabeledTuple(entry_types)
      }

      TypeExprKind::Func(param, ret) => {
        self.analyze_type_expr(param);
        self.analyze_type_expr(ret);

        let param_type = param.typ.clone();
        let return_type = ret.typ.clone();

        ValueType::Func(vec![param_type], Box::new(return_type))
      }
    };

    node.typ = typ;
  }
}

impl<'a> VisitorMut for Analyzer<'a> {
  fn enter_module(&mut self, node: &mut ModuleNode) {
    // First thing, go through the top-level statements and collect the definitions,
    // since they may be used before they are defined.
    for statement in &mut node.body {
      match &mut statement.kind {
        TopLevelStatementKind::Const(const_node) => self.collect_const(const_node),

        TopLevelStatementKind::Def(def_node) => self.collect_def(
          def_node.pos,
          &mut def_node.generic_type_constraints,
          &mut def_node.kind,
          &def_node.return_type,
        ),

        TopLevelStatementKind::IntrinsicDef(def_node) => self.collect_def(
          def_node.pos,
          &mut def_node.generic_type_constraints,
          &mut def_node.kind,
          &def_node.return_type,
        ),

        TopLevelStatementKind::TypeDef(type_def_node) => self.collect_type_def(type_def_node),

        TopLevelStatementKind::IntrinsicTypeDef(type_def_node) => {
          self.collect_intrinsic_type_def(type_def_node)
        }

        _ => {
          // Other kinds handled below
        }
      }
    }
  }

  fn enter_top_level_statement(&mut self, node: &mut TopLevelStatementNode) {
    match &mut node.kind {
      TopLevelStatementKind::Def(def_node) => {
        self.analyze_def(def_node);
      }

      TopLevelStatementKind::IntrinsicDef(def_node) => {
        self.analyze_intrinsic_def(def_node);
      }

      TopLevelStatementKind::Let(let_node) => {
        self.analyze_expr(&mut let_node.value);
        self.destructure_pattern(&mut let_node.pattern, &mut let_node.value.typ);
      }

      TopLevelStatementKind::Expr(expr) => {
        self.analyze_expr(expr);
      }

      _ => {
        // Other kinds handled above
      }
    }
  }
}
