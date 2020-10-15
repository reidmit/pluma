use crate::analysis_error::{AnalysisError, AnalysisErrorKind};
use crate::binding::*;
use crate::scope::*;
use crate::type_utils;
use ast::*;
use diagnostics::*;
use std::collections::HashMap;
use std::iter::Iterator;
use visitor::*;

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

  fn constraint_to_required_fields(
    &mut self,
    constraint: &TypeConstraint,
  ) -> HashMap<String, Binding> {
    match &constraint {
      TypeConstraint::NamedTrait(name) => {
        let value_type = ValueType::Named(name.to_owned());
        let type_binding = self.scope.get_type_binding(&value_type).unwrap();
        type_binding.fields()
      }
      _ => todo!("other constraint flavors"),
    }
  }

  fn type_to_field_types(&mut self, typ: &ValueType) -> HashMap<String, ValueType> {
    let mut field_types = HashMap::new();

    match typ {
      ValueType::UnlabeledTuple(entries) => {
        for i in 0..entries.len() {
          field_types.insert(format!("{}", i), entries.get(i).unwrap().clone());
        }
      }

      ValueType::LabeledTuple(entries) => {
        for (label, typ) in entries {
          field_types.insert(format!("{}", label), typ.clone());
        }
      }

      ValueType::Named(..) => {
        let type_binding = self.scope.get_type_binding(&typ).unwrap();
        return type_binding.field_types();
      }

      _ => {}
    }

    field_types
  }

  fn compatible_types(&mut self, expected: &ValueType, actual: &ValueType) -> bool {
    match expected {
      ValueType::Constrained(constraint) => {
        // For constrained types, we need to make sure the actual type has each
        // of the fields and methods defined by the constraint (with compatible types)

        let required_fields = self.constraint_to_required_fields(constraint);
        let actual_fields = self.type_to_field_types(actual);

        for (field_name, field_binding) in required_fields {
          match actual_fields.get(&field_name) {
            None => {
              println!("missing trait field! {}", field_name);
              return false;
            }

            Some(actual_type) => {
              if !self.compatible_types(&field_binding.typ, actual_type) {
                println!("incompatible field types! {}", field_name);
                return false;
              }
            }
          }
        }

        true
      }
      _ => expected == actual,
    }
  }

  fn collect_def(
    &mut self,
    pos: Position,
    generic_type_constraints: &mut GenericTypeConstraints,
    kind: &mut DefKind,
    return_type: &mut Option<TypeExprNode>,
  ) {
    // First, go through the type constraints (where clause); this will help us know
    // if any of the types in the def signature are generics.
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

      constraints_map.insert(constraint_name.name.clone(), constraint);
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
              param_types.push(ValueType::Constrained(constraint.clone()));

              part_type
                .to_type_identifier_mut()
                .add_constraint(constraint.clone())
            } else {
              param_types.push(param_type);
            }
          } else {
            param_types.push(param_type);
          }
        }

        let func_return_type = match return_type {
          Some(ret) => {
            let mut func_return_type = type_utils::type_expr_to_value_type(&ret);

            if let ValueType::Named(name) = &func_return_type {
              if let Some(constraint) = constraints_map.get(name) {
                func_return_type = ValueType::Constrained(constraint.clone());

                ret
                  .to_type_identifier_mut()
                  .add_constraint(constraint.clone());
              }
            }

            func_return_type
          }
          None => ValueType::Nothing,
        };

        let def_type = ValueType::Func(param_types, Box::new(func_return_type));
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
    }
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

      TypeDefKind::Struct { inner } => {
        let fields = type_utils::type_expr_to_struct_fields(inner);

        self.scope.add_type_binding(
          typ.clone(),
          TypeBindingKind::Struct { fields },
          node.name.pos,
        );

        let inner_type = type_utils::type_expr_to_value_type(inner);
        let constructor_type = ValueType::Func(vec![inner_type], Box::new(typ));

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

        if !self.compatible_types(&constructor_type, &actual_type) {
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

    for stmt in &mut node.body {
      self.analyze_statement(stmt);

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

          if !self.compatible_types(&param_type, &given_type) {
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
    let mut param_types = Vec::new();
    let mut return_type = ValueType::Nothing;

    match &mut node.kind {
      DefKind::Function { signature } => {
        let params = &node.block.params;

        if params.len() > signature.len() {
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

          param_types.push(part_type.typ.clone());
        }
      }

      DefKind::Method {
        receiver,
        signature,
      } => {
        let receiver_type = self.analyze_type_identifier(receiver);

        param_types.push(receiver_type);

        for (_part_name, part_type) in signature {
          self.analyze_type_expr(part_type);

          param_types.push(part_type.typ.clone());
        }
      }
    }

    if let Some(type_expr) = &mut node.return_type {
      println!("ret type_expr: {:#?}", type_expr);
      self.analyze_type_expr(type_expr);

      return_type = type_expr.typ.clone();
    }

    self.scope.enter();

    if node.block.params.is_empty() {
      let pos = (node.block.pos.0, node.block.pos.0);

      let mut i = 0;
      for param_type in param_types {
        self
          .scope
          .add_binding(BindingKind::Param, format!("${}", i), param_type, pos);

        i += 1;
      }
    } else {
      let mut i = 0;
      for pattern in &mut node.block.params {
        let param_type = param_types.get(i).unwrap();
        self.destructure_pattern(pattern, param_type);

        i += 1;
      }
    }

    let mut block_return_type = ValueType::Nothing;
    let mut block_return_pos = node.block.pos;

    for stmt in &mut node.block.body {
      self.analyze_statement(stmt);

      if let StatementKind::Expr(expr) = &stmt.kind {
        block_return_type = expr.typ.clone();
        block_return_pos = expr.pos;
      }
    }

    if !self.compatible_types(&return_type, &block_return_type) {
      self.error(AnalysisError {
        pos: block_return_pos,
        kind: AnalysisErrorKind::ReturnTypeMismatch {
          expected: return_type,
          actual: block_return_type,
        },
      })
    }

    let results = self.scope.exit();
    self.check_results(results);
  }

  fn analyze_expr(&mut self, node: &mut ExprNode) {
    match &mut node.kind {
      ExprKind::Assignment { left: _, right: _ } => {
        // let existing_binding = self.scope.get_binding(&left.name);

        // if let Some(binding) = existing_binding {
        //   let current_type = binding.typ.clone();
        //   let new_type = right.typ.clone();

        //   if !self.compatible_types(&current_type, &new_type) {
        //     self.error(AnalysisError {
        //       pos: right.pos,
        //       kind: AnalysisErrorKind::ReassignmentTypeMismatch {
        //         expected: current_type,
        //         actual: new_type,
        //       },
        //     })
        //   }
        // }
      }

      ExprKind::BinaryOperation { op, left, right } => {
        self.analyze_expr(left);
        self.analyze_expr(right);

        match op.kind {
          OperatorKind::Add => {
            // self.ensure_satisfies_trait(left, "Add");
            // self.ensure_satisfies_trait(right, "Add");
            // self.ensure_same_type(left, right);
          }

          _ => todo!(),
        }

        // let receiver_type_binding = match self.scope.get_type_binding(&left.typ) {
        //   Some(binding) => binding,
        //   _ => return,
        // };

        // let method_name_parts = vec!["$".to_owned(), op.name.clone(), "$".to_owned()];

        // if let Some(method_type) = receiver_type_binding.methods.get(&method_name_parts) {
        //   let param_types = method_type.func_param_types();
        //   let first_param_type = param_types.first().unwrap();

        //   node.typ = method_type.func_return_type();

        //   if !self.compatible_types(&right.typ, first_param_type) {
        //     self.error(AnalysisError {
        //       pos: right.pos,
        //       kind: AnalysisErrorKind::ParameterTypeMismatch {
        //         expected: first_param_type.clone(),
        //         actual: right.typ.clone(),
        //       },
        //     })
        //   }
        // } else {
        //   self.error(AnalysisError {
        //     pos: op.pos,
        //     kind: AnalysisErrorKind::UndefinedBinaryOperatorForType {
        //       op_name: op.name.clone(),
        //       receiver_type: left.typ.clone(),
        //       param_type: right.typ.clone(),
        //     },
        //   })
        // }
      }

      ExprKind::Block { block } => node.typ = self.analyze_block(block),

      ExprKind::Call { call } => node.typ = self.analyze_call(call),

      ExprKind::EmptyTuple => node.typ = ValueType::Nothing,

      ExprKind::Access { receiver, property } => {
        self.analyze_expr(receiver);

        match &property.kind {
          ExprKind::Identifier { ident } => {
            // If an identifier, then the receiver must have a field or method with this name.

            let receiver_type_fields = self.type_to_field_types(&receiver.typ);

            match receiver_type_fields.get(&ident.name) {
              Some(field_typ) => node.typ = field_typ.clone(),

              // TODO check methods?
              None => self.error(AnalysisError {
                pos: property.pos,
                kind: AnalysisErrorKind::UndefinedFieldForType {
                  field_name: ident.name.clone(),
                  receiver_type: receiver.typ.clone(),
                },
              }),
            }
          }

          ExprKind::MultiPartIdentifier { parts } => {
            let method_name_parts = parts
              .iter()
              .map(|n| n.name.clone())
              .collect::<Vec<String>>();

            let receiver_type_binding = match self.scope.get_type_binding(&receiver.typ) {
              Some(binding) => binding,
              _ => return,
            };

            if let Some(method_type) = receiver_type_binding.methods.get(&method_name_parts) {
              node.typ = method_type.clone();
            } else {
              let pos = (parts.first().unwrap().pos.0, parts.last().unwrap().pos.1);

              self.error(AnalysisError {
                pos,
                kind: AnalysisErrorKind::UndefinedMethodForType {
                  method_name_parts,
                  receiver_type: receiver.typ.clone(),
                },
              })
            }
          }

          _ => todo!(),
        }
      }

      ExprKind::Grouping { inner } => {
        self.analyze_expr(inner);
        node.typ = inner.typ.clone();
      }

      ExprKind::Identifier { ident } => {
        node.typ = self.analyze_identifier(ident);
      }

      ExprKind::Interpolation { parts } => {
        let string_type = ValueType::Named("String".to_owned());

        for part in parts {
          if !self.compatible_types(&string_type, &part.typ) {
            self.error(AnalysisError {
              pos: part.pos,
              kind: AnalysisErrorKind::TypeMismatchInStringInterpolation(part.typ.clone()),
            })
          }
        }

        node.typ = string_type;
      }

      ExprKind::Literal { literal } => node.typ = self.analyze_literal(literal),

      ExprKind::Match { match_ } => {
        let _subject_type = &match_.subject.typ;
        let mut case_type: Option<ValueType> = None;

        for case in &match_.cases {
          if let Some(expected_case_type) = &case_type {
            let actual_case_type = &case.body.typ;

            if !self.compatible_types(&expected_case_type, &actual_case_type) {
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

      ExprKind::MultiPartIdentifier { parts } => {
        let names = parts
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

        if !self.compatible_types(asserted_type, expr_type) {
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

      // ExprKind::UnaryOperation { op, right } => {
      //   let receiver_type_binding = match self.scope.get_type_binding(&right.typ) {
      //     Some(binding) => binding,
      //     _ => return,
      //   };

      //   let method_name_parts = vec![op.name.clone(), "$".to_owned()];

      //   if let Some(method_type) = receiver_type_binding.methods.get(&method_name_parts) {
      //     node.typ = method_type.func_return_type();
      //   } else {
      //     self.error(AnalysisError {
      //       pos: op.pos,
      //       kind: AnalysisErrorKind::UndefinedUnaryOperatorForType {
      //         op_name: op.name.clone(),
      //         receiver_type: right.typ.clone(),
      //       },
      //     })
      //   }
      // }
      ExprKind::UnlabeledTuple { entries } => {
        let mut entry_types = Vec::new();

        for entry in entries {
          self.analyze_expr(entry);
          entry_types.push(entry.typ.clone());
        }

        node.typ = ValueType::UnlabeledTuple(entry_types);
      }

      ExprKind::LabeledTuple { entries } => {
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
    }

    if let Some(return_type) = &mut node.return_type {
      self.analyze_type_expr(return_type);
    }
  }

  fn analyze_let(&mut self, node: &mut LetNode) {
    self.analyze_expr(&mut node.value);

    self.destructure_pattern(&mut node.pattern, &mut node.value.typ);
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

  fn analyze_statement(&mut self, node: &mut StatementNode) {
    match &mut node.kind {
      StatementKind::Expr(expr_node) => self.analyze_expr(expr_node),

      StatementKind::Let(let_node) => self.analyze_let(let_node),
    }
  }

  fn analyze_type_identifier(&mut self, node: &mut TypeIdentifierNode) -> ValueType {
    if let Some(constraints) = &node.constraints {
      // TODO: more than just the first here?
      return ValueType::Constrained(constraints.first().unwrap().clone());
    }

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
        TopLevelStatementKind::Def(def_node) => self.collect_def(
          def_node.pos,
          &mut def_node.generic_type_constraints,
          &mut def_node.kind,
          &mut def_node.return_type,
        ),

        TopLevelStatementKind::IntrinsicDef(def_node) => self.collect_def(
          def_node.pos,
          &mut def_node.generic_type_constraints,
          &mut def_node.kind,
          &mut def_node.return_type,
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
      TopLevelStatementKind::Def(def_node) => self.analyze_def(def_node),

      TopLevelStatementKind::IntrinsicDef(def_node) => self.analyze_intrinsic_def(def_node),

      TopLevelStatementKind::Let(let_node) => self.analyze_let(let_node),

      TopLevelStatementKind::Expr(expr) => self.analyze_expr(expr),

      _ => {
        // Other kinds handled above
      }
    }
  }
}
