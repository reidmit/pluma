use crate::ast::*;
use crate::binding::*;
use crate::diagnostic::*;
use crate::errors::*;
use crate::location::Range;
use crate::module::{Module, ModuleExports};
use crate::types::*;
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::path::PathBuf;
use AnalysisErrorKind::*;

enum VariantResolution {
	Found(String, Vec<Type>),
	NotFound,
	Ambiguous,
}

pub struct Analyzer<'compiler> {
	module_name: Option<String>,
	module_path: Option<PathBuf>,
	diagnostics: &'compiler mut Vec<Diagnostic>,
	value_scopes: Vec<HashMap<String, ValueBinding>>,
	type_scope: HashMap<String, TypeBinding>,
	// Enum definitions visible during analysis, keyed by the *qualified*
	// enum name (`<defining-module>.<enum-name>`). Both locally-defined
	// enums and imported ones are stored here under that qualified key so
	// that `Type::Enum(qualified-name)` lookups work uniformly.
	enum_defs: HashMap<String, Vec<(String, Vec<Type>)>>,
	// Imports: local namespace name (e.g. `math` from `use math` or `utils`
	// from `use sub.utils`) -> that module's full exports.
	imports: HashMap<String, ModuleExports>,
	// The fully-qualified name of each imported module, keyed by the local
	// namespace name. `use a.b.utils as u` produces `u -> a.b.utils`.
	import_qualified: HashMap<String, String>,
	next_type_var_id: usize,
}

impl<'compiler> Analyzer<'compiler> {
	/// Creates a new `Analyzer`. Takes a mutable list of diagnostics
	/// to which any analyis errors/warnings will be appended.
	pub fn new(diagnostics: &'compiler mut Vec<Diagnostic>) -> Self {
		Self {
			module_name: None,
			module_path: None,
			diagnostics,
			value_scopes: Vec::new(),
			type_scope: HashMap::new(),
			enum_defs: HashMap::new(),
			imports: HashMap::new(),
			import_qualified: HashMap::new(),
			next_type_var_id: 0,
		}
	}

	/// Runs analysis over a parsed module. The AST will be annotated
	/// with inferred types (hence the mutability).
	pub fn analyze(&mut self, module: &mut Module) {
		self.module_name = Some(module.module_name.clone());
		self.module_path = Some(module.module_path.clone());

		// TODO: We're adding the builtin types here, but there must be a better way
		self.add_type_binding("int".into(), Type::Int, Range::collapsed(0, 0));
		self.add_type_binding("bool".into(), Type::Bool, Range::collapsed(0, 0));
		self.add_type_binding("string".into(), Type::String, Range::collapsed(0, 0));
		self.add_type_binding("regex".into(), Type::Regex, Range::collapsed(0, 0));
		self.add_type_binding("float".into(), Type::Float, Range::collapsed(0, 0));

		// Seed enum_defs with imported enums under their canonical
		// `<defining-module>.<enum-name>` keys, so variant resolution and
		// exhaustiveness checks can see them.
		let imported_enums: Vec<(String, String, Vec<(String, Vec<Type>)>)> = self
			.imports
			.iter()
			.flat_map(|(local_name, exports)| {
				let qualified_module = self
					.import_qualified
					.get(local_name)
					.cloned()
					.unwrap_or_else(|| local_name.clone());
				exports.enums.iter().map(move |(enum_name, variants)| {
					(
						qualified_module.clone(),
						enum_name.clone(),
						variants.clone(),
					)
				})
			})
			.collect();
		for (qualified_module, enum_name, variants) in imported_enums {
			let qualified = format!("{}.{}", qualified_module, enum_name);
			self.enum_defs.insert(qualified, variants);
		}

		self.enter_scope();

		// the three basic phases of analysis!
		let substitution = if let Some(ast) = &mut module.ast {
			// 1. generate constraints based on AST (and also fill in any
			//    types we can infer without constraints, like for literals)
			let constraints = self.constrain(ast);

			// 2. find a solution that unifies all the constraints
			let substitution = self.unify(&constraints);

			// 3. apply the solution to the AST, filling in type variables
			//    that we generated in phase 1
			self.annotate(ast, &substitution);

			Some(substitution)
		} else {
			None
		};

		// Build the module's exports. Values come from the inferred types of
		// each top-level expr def. Aliases are resolved by applying the
		// substitution to the alias's type binding. Enums are pulled from
		// enum_defs by qualified name and re-keyed by bare name.
		let mut exports = ModuleExports::default();
		if let Some(ast) = &module.ast {
			for def in &ast.body {
				match &def.kind {
					DefinitionKind::Expr(expr) => {
						exports
							.values
							.insert(def.name.name.clone(), expr.ty.clone());
					}
					DefinitionKind::Alias(_) => {
						// Alias types are exported both as types (for use in
						// type positions like `module.alias-name`) and as
						// constructor functions (for use in value positions
						// like `module.alias-name { ... }`).
						if let Some(binding) = self.type_scope.get(&def.name.name) {
							let resolved = match &substitution {
								Some(s) => s.apply_to_type(&binding.ty),
								None => binding.ty.clone(),
							};
							exports
								.aliases
								.insert(def.name.name.clone(), resolved.clone());
							exports.values.insert(
								def.name.name.clone(),
								Type::Fun(vec![resolved.clone()], Box::new(resolved)),
							);
						}
					}
					DefinitionKind::Enum(_) => {
						let qualified = format!("{}.{}", module.module_name, def.name.name);
						if let Some(variants) = self.enum_defs.get(&qualified) {
							exports
								.enums
								.insert(def.name.name.clone(), variants.clone());
						}
					}
				}
			}
		}
		module.exports = Some(exports);
	}

	pub fn set_imports(
		&mut self,
		imports: HashMap<String, ModuleExports>,
		import_qualified: HashMap<String, String>,
	) {
		self.imports = imports;
		self.import_qualified = import_qualified;
	}

	fn diagnostic(&mut self, range: Option<Range>, diag: Diagnostic) {
		let mut diag = diag;

		if let Some(range) = range {
			diag = diag.with_span(range);
		}

		if let Some(module_name) = &self.module_name {
			diag = diag.with_module(module_name.clone(), self.module_path.clone().unwrap())
		}

		self.diagnostics.push(diag)
	}

	fn warning(&mut self, range: Range, kind: AnalysisErrorKind) {
		self.diagnostic(Some(range), Diagnostic::warning(AnalysisError { kind }));
	}

	fn error(&mut self, range: Range, kind: AnalysisErrorKind) {
		self.diagnostic(Some(range), Diagnostic::error(AnalysisError { kind }));
	}

	fn add_value_binding(&mut self, name: String, ty_scheme: Scheme, range: Range) {
		let current_level = self.value_scopes.last_mut().expect("no current scope");

		current_level.insert(
			name,
			ValueBinding {
				ty_scheme,
				ref_count: 0,
				range,
			},
		);
	}

	fn get_value_binding(&mut self, name: &String) -> Option<&ValueBinding> {
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

	fn leave_scope(&mut self) {
		if let Some(exited_level) = self.value_scopes.pop() {
			for (name, binding) in exited_level {
				if binding.ref_count == 0 && !name.starts_with("_") {
					self.warning(binding.range, UnusedBinding { name });
				}
			}
		}
	}

	fn add_type_binding(&mut self, name: String, ty: Type, range: Range) {
		self.type_scope.insert(
			name,
			TypeBinding {
				ty,
				ref_count: 0,
				_range: range,
			},
		);
	}

	fn get_type_binding(&mut self, name: &String) -> Option<&TypeBinding> {
		if let Some(binding) = self.type_scope.get_mut(name) {
			binding.ref_count += 1;

			return Some(binding);
		}

		None
	}

	fn constrain(&mut self, module: &mut ModuleNode) -> Vec<Constraint> {
		let mut constraints = Vec::new();
		let mut schemes = Vec::new();
		let mut type_def_vars = Vec::new();

		// first, do a shallow pass to annotate all top-level defs and add them to the scope,
		// so that they can be referenced anywhere within the bodies of other defs
		for definition in &mut module.body {
			definition.ty = self.new_type_var();

			match &mut definition.kind {
				DefinitionKind::Expr(_) => {
					// Similar to lets, we generate a new type scheme for the definition body.
					// This allows defs to be polymorphic (e.g. `def identity fun x { x }`) -
					// these can be instantiated later into concrete types when used.
					let type_scheme = self.new_type_scheme_var();

					self.add_value_binding(
						definition.name.name.clone(),
						type_scheme.clone(),
						definition.name.range,
					);

					schemes.push(type_scheme);
				}

				DefinitionKind::Alias(_) => {
					// Add a type binding for the type defined here...
					let type_var = self.new_type_var();
					self.add_type_binding(
						definition.name.name.clone(),
						type_var.clone(),
						definition.name.range,
					);
					type_def_vars.push(type_var);

					// And also a value binding for the constructor function!
					let type_scheme = self.new_type_scheme_var();
					self.add_value_binding(
						definition.name.name.clone(),
						type_scheme.clone(),
						definition.name.range,
					);
					schemes.push(type_scheme);
				}

				DefinitionKind::Enum(_) => {
					// Enums are nominal: bind the enum type directly. No value binding —
					// the bare name isn't a value, it's only used as a namespace for
					// variant access (e.g. `color.red`), which is resolved via enum_defs.
					// The canonical type name is qualified with the defining module so
					// same-named enums from different modules don't unify.
					let qualified = format!(
						"{}.{}",
						self.module_name.as_ref().unwrap(),
						definition.name.name
					);
					self.add_type_binding(
						definition.name.name.clone(),
						Type::Enum(qualified),
						definition.name.range,
					);
				}
			}
		}

		// then, we go through and generate constraints from the defs
		let mut scheme_index = 0;
		let mut type_def_index = 0;

		for definition in &mut module.body {
			match &mut definition.kind {
				DefinitionKind::Expr(expr) => {
					self.constrain_expr(expr, &mut constraints);

					let scheme = schemes.get(scheme_index).unwrap().clone();
					constraints.push(Constraint::Gen(scheme, expr.ty.clone()));
					scheme_index += 1;
				}

				DefinitionKind::Alias(type_expr) => {
					let ty = self.type_expr_to_type(type_expr, &mut constraints);
					let type_var = type_def_vars.get(type_def_index).unwrap().clone();
					constraints.push(eq_constraint(type_var.clone(), ty.clone()));
					type_def_index += 1;

					let scheme = schemes.get(scheme_index).unwrap().clone();
					let constructor_type = Type::Fun(vec![ty.clone()], type_var.clone().into());
					constraints.push(Constraint::Gen(scheme, constructor_type));
					scheme_index += 1;
				}

				DefinitionKind::Enum(enum_node) => {
					let variants: Vec<(String, Vec<Type>)> = enum_node
						.variants
						.iter()
						.map(|variant| {
							let params = variant
								.params
								.as_ref()
								.map(|params| {
									params
										.iter()
										.map(|p| self.type_expr_to_type(p, &mut constraints))
										.collect()
								})
								.unwrap_or_default();
							(variant.name.name.clone(), params)
						})
						.collect();

					let qualified = format!(
						"{}.{}",
						self.module_name.as_ref().unwrap(),
						definition.name.name
					);
					self.enum_defs.insert(qualified, variants);
				}
			}
		}

		constraints
	}

	fn type_expr_to_type(
		&mut self,
		type_expr: &TypeExprNode,
		constraints: &mut Vec<Constraint>,
	) -> Type {
		match &type_expr.kind {
			TypeExprKind::EmptyTuple => Type::Nothing,
			TypeExprKind::Grouping(inner) => self.type_expr_to_type(inner, constraints),
			TypeExprKind::Tuple(entries) => Type::Tuple(
				entries
					.into_iter()
					.map(|e| self.type_expr_to_type(e, constraints))
					.collect(),
			),
			TypeExprKind::Record(fields) => Type::Record(
				fields
					.into_iter()
					.map(|(name, f)| (name.name.clone(), self.type_expr_to_type(f, constraints)))
					.collect(),
			),
			TypeExprKind::Func(params, ret) => Type::Fun(
				params
					.into_iter()
					.map(|p| self.type_expr_to_type(p, constraints))
					.collect(),
				self.type_expr_to_type(ret, constraints).into(),
			),
			TypeExprKind::Single(type_ident) => {
				// `module.TypeName`: look up the type in the named import.
				if let Some(module) = &type_ident.module {
					if let Some(exports) = self.imports.get(&module.name).cloned() {
						if let Some(variants) = exports.enums.get(&type_ident.name).cloned() {
							let _ = variants; // consumed for membership only
							let qualified_module = self
								.import_qualified
								.get(&module.name)
								.cloned()
								.unwrap_or_else(|| module.name.clone());
							return Type::Enum(format!("{}.{}", qualified_module, type_ident.name));
						}

						if let Some(alias_ty) = exports.aliases.get(&type_ident.name) {
							return alias_ty.clone();
						}

						self.error(
							type_ident.range,
							NameNotBound {
								name: format!("{}.{}", module.name, type_ident.name),
							},
						);
						return Type::Unknown;
					}

					self.error(
						module.range,
						NameNotBound {
							name: module.name.clone(),
						},
					);
					return Type::Unknown;
				}

				match &type_ident.name[..] {
					"string" => return Type::String,
					"int" => return Type::Int,
					"float" => return Type::Float,
					"bool" => return Type::Bool,
					"regex" => return Type::Regex,
					_ => {
						if let Some(binding) = self.get_type_binding(&type_ident.name) {
							return binding.ty.clone();
						}
					}
				}

				self.error(
					type_ident.range,
					NameNotBound {
						name: type_ident.name.clone(),
					},
				);

				Type::Unknown
			}
		}
	}

	fn constrain_expr(&mut self, expr: &mut ExprNode, constraints: &mut Vec<Constraint>) {
		use Constraint::*;

		match &mut expr.kind {
			// For each of these, we don't bother introducing a new type var and generating
			// a constraint that the var is eq to the known concrete type. We could do that
			// (the algorithm would handle it fine), but assigning the concrete type directly
			// is nicer to look at and saves a couple steps.
			ExprKind::EmptyTuple => expr.ty = Type::Nothing,
			ExprKind::Regex(..) => expr.ty = Type::Regex,
			ExprKind::Literal(literal) => match &mut literal.kind {
				LiteralKind::Bool(..) => expr.ty = Type::Bool,
				LiteralKind::String(..) => expr.ty = Type::String,
				LiteralKind::FloatDecimal(..) => expr.ty = Type::Float,
				LiteralKind::IntDecimal(..)
				| LiteralKind::IntHex(..)
				| LiteralKind::IntBinary(..)
				| LiteralKind::IntOctal(..) => expr.ty = Type::Int,
			},

			ExprKind::Identifier(ident) => {
				if let Some(binding) = self.get_value_binding(&ident.name) {
					return match &binding.ty_scheme {
						Scheme::Forall(_, ty) => {
							expr.ty = ty.clone();
						}

						Scheme::Var(var) => {
							let var = *var;
							let expr_ty = self.new_type_var();
							expr.ty = expr_ty.clone();
							constraints.push(Inst(var, expr_ty))
						}
					};
				};

				self.error(
					ident.range,
					NameNotBound {
						name: ident.name.clone(),
					},
				);

				expr.ty = Type::Unknown;
			}

			ExprKind::Interpolation(parts) => {
				for part in parts {
					self.constrain_expr(part, constraints);

					// each part must have type string
					constraints.push(eq_constraint(part.ty.clone(), Type::String).at(part.range));
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

				constraints.push(eq_constraint(expr.ty.clone(), Type::Tuple(element_types)).at(expr.range))
			}

			ExprKind::Record(fields) => {
				expr.ty = self.new_type_var();

				let mut field_types = Vec::new();

				for (field_name, field_value) in fields {
					self.constrain_expr(field_value, constraints);
					field_types.push((field_name.name.clone(), field_value.ty.clone()));
				}

				constraints.push(eq_constraint(expr.ty.clone(), Type::Record(field_types)).at(expr.range))
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
						constraints.push(eq_constraint(left.ty.clone(), Type::Int).at(left.range));
						constraints.push(eq_constraint(right.ty.clone(), Type::Int).at(right.range));
					}

					Operator::LogicalAnd | Operator::LogicalOr => {
						expr.ty = Type::Bool;
						constraints.push(eq_constraint(left.ty.clone(), Type::Bool).at(left.range));
						constraints.push(eq_constraint(right.ty.clone(), Type::Bool).at(right.range));
					}

					Operator::FieldAccess => unreachable!("handled separately"),

					other => {
						// todo :----)
						println!("found unhandled binary op: {}", other)
					}
				}
			}

			ExprKind::Fun(FunNode { params, body, .. }) => {
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
							param.ident.range,
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
					.at(expr.range),
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
					.at(expr.range),
				);
			}

			ExprKind::Let(LetNode { name, value, .. }) => {
				// visit the value (expression after the `=`), and collect constraints:
				self.constrain_expr(value, constraints);

				// add a new type scheme to the context with a new var:
				// If the value's type is already fully concrete (no free type vars),
				// bind monomorphically so subsequent uses see the resolved type at
				// constraint-gen time. Otherwise defer via Gen/Inst so the binding
				// can be polymorphic.
				if value.ty.free_vars().is_empty() {
					self.add_value_binding(
						name.name.clone(),
						Scheme::Forall(vec![], value.ty.clone()),
						name.range,
					);
				} else {
					let type_scheme = self.new_type_scheme_var();
					self.add_value_binding(name.name.clone(), type_scheme.clone(), name.range);
					constraints.push(Gen(type_scheme, value.ty.clone()));
				}

				// let expressions always evaluate to ()
				expr.ty = Type::Nothing;
			}

			ExprKind::ElementAccess { receiver, index } => {
				// this expr gets a fresh type var
				expr.ty = self.new_type_var();

				self.constrain_expr(receiver, constraints);

				// we know that receiver is a "partial tuple": at given index, it
				// must have a value of the type of this expr
				constraints.push(
					eq_constraint(
						receiver.ty.clone(),
						Type::PartialTuple(*index, expr.ty.clone().into()),
					)
					.at(expr.range),
				)
			}

			ExprKind::FieldAccess { receiver, field } => {
				// Cross-module variant access: `module.enum-name.variant`.
				// Match the chained-FieldAccess shape so we resolve before the
				// inner receiver gets recursed into as a regular field access.
				if let ExprKind::FieldAccess {
					receiver: outer_recv,
					field: enum_field,
				} = &receiver.kind
				{
					if let ExprKind::Identifier(module_ident) = &outer_recv.kind {
						if let Some(exports) = self.imports.get(&module_ident.name).cloned() {
							if let Some(variants) = exports.enums.get(&enum_field.name).cloned() {
								let qualified_module = self
									.import_qualified
									.get(&module_ident.name)
									.cloned()
									.unwrap_or_else(|| module_ident.name.clone());
								let enum_ty = Type::Enum(format!("{}.{}", qualified_module, enum_field.name));
								receiver.ty = enum_ty.clone();
								if let ExprKind::FieldAccess {
									receiver: inner, ..
								} = &mut receiver.kind
								{
									inner.ty = Type::Unknown;
								}

								match variants.iter().find(|(n, _)| n == &field.name) {
									Some((_, params)) if params.is_empty() => {
										expr.ty = enum_ty;
									}
									Some((_, params)) => {
										expr.ty = Type::Fun(params.clone(), enum_ty.into());
									}
									None => {
										self.error(
											field.range,
											EnumVariantNotPresent {
												variant: field.name.clone(),
												ty: enum_ty,
											},
										);
										expr.ty = Type::Unknown;
									}
								}
								return;
							}
						}
					}
				}

				// Module namespace access: `module-name.def`. If the receiver
				// is a bare ident that matches an imported module, look up
				// the field in that module's exported values.
				if let ExprKind::Identifier(ident) = &receiver.kind {
					if let Some(exports) = self.imports.get(&ident.name).cloned() {
						match exports.values.get(&field.name) {
							Some(ty) => {
								receiver.ty = Type::Unknown;
								expr.ty = self.instantiate(ty);
							}
							None => {
								self.error(
									field.range,
									NameNotBound {
										name: format!("{}.{}", ident.name, field.name),
									},
								);
								expr.ty = Type::Unknown;
							}
						}
						return;
					}
				}

				// Local variant access: `EnumName.variant`. The receiver is a
				// bare ident that resolves (via type_scope) to a known enum.
				if let ExprKind::Identifier(ident) = &receiver.kind {
					let qualified_enum = self.type_scope.get(&ident.name).and_then(|binding| {
						if let Type::Enum(name) = &binding.ty {
							Some(name.clone())
						} else {
							None
						}
					});

					if let Some(qualified) = qualified_enum {
						if let Some(variants) = self.enum_defs.get(&qualified).cloned() {
							let enum_ty = Type::Enum(qualified);
							receiver.ty = enum_ty.clone();

							match variants.iter().find(|(n, _)| n == &field.name) {
								Some((_, params)) if params.is_empty() => {
									expr.ty = enum_ty;
								}
								Some((_, params)) => {
									expr.ty = Type::Fun(params.clone(), enum_ty.into());
								}
								None => {
									self.error(
										field.range,
										EnumVariantNotPresent {
											variant: field.name.clone(),
											ty: enum_ty,
										},
									);
									expr.ty = Type::Unknown;
								}
							}

							return;
						}
					}
				}

				// this expr gets a fresh type var
				expr.ty = self.new_type_var();

				self.constrain_expr(receiver, constraints);

				// we know that receiver is a "partial record": at given field name, it
				// must have a value of the type of this expr
				constraints.push(
					eq_constraint(
						receiver.ty.clone(),
						Type::PartialRecord(field.name.clone(), expr.ty.clone().into()),
					)
					.at(expr.range),
				)
			}

			ExprKind::If(IfNode {
				subject,
				pattern,
				body,
				..
			}) => {
				self.constrain_expr(subject, constraints);

				self.enter_scope();
				self.constrain_pattern(pattern, subject.ty.clone(), constraints);
				for body_expr in body.iter_mut() {
					self.constrain_expr(body_expr, constraints);
				}
				self.leave_scope();

				// single-armed if always evaluates to nothing
				expr.ty = Type::Nothing;
			}

			ExprKind::While(WhileNode {
				subject,
				pattern,
				body,
				..
			}) => {
				self.constrain_expr(subject, constraints);

				self.enter_scope();
				self.constrain_pattern(pattern, subject.ty.clone(), constraints);
				for body_expr in body.iter_mut() {
					self.constrain_expr(body_expr, constraints);
				}
				self.leave_scope();

				expr.ty = Type::Nothing;
			}

			ExprKind::When(WhenNode { subject, cases, .. }) => {
				self.constrain_expr(subject, constraints);
				expr.ty = self.new_type_var();

				for case in cases.iter_mut() {
					self.enter_scope();
					self.constrain_pattern(&mut case.pattern, subject.ty.clone(), constraints);

					let mut case_ty = Type::Nothing;
					for body_expr in case.body.iter_mut() {
						self.constrain_expr(body_expr, constraints);
						case_ty = body_expr.ty.clone();
					}
					self.leave_scope();

					constraints.push(eq_constraint(expr.ty.clone(), case_ty).at(case.range));
				}
			}

			_ => {
				// todo :---)
			}
		}
	}

	fn constrain_pattern(
		&mut self,
		pattern: &mut PatternNode,
		subject_ty: Type,
		constraints: &mut Vec<Constraint>,
	) {
		match &mut pattern.kind {
			PatternKind::Underscore => {}

			PatternKind::Literal(literal) => {
				let lit_ty = match &literal.kind {
					LiteralKind::Bool(..) => Type::Bool,
					LiteralKind::String(..) => Type::String,
					LiteralKind::FloatDecimal(..) => Type::Float,
					LiteralKind::IntDecimal(..)
					| LiteralKind::IntHex(..)
					| LiteralKind::IntBinary(..)
					| LiteralKind::IntOctal(..) => Type::Int,
				};
				constraints.push(eq_constraint(subject_ty, lit_ty).at(pattern.range));
			}

			PatternKind::Identifier(ident) => {
				// A bare ident might be a nullary variant match. Use the subject
				// type to disambiguate; otherwise require global uniqueness.
				match self.resolve_variant_pattern(ident, &subject_ty, /* nullary_only */ true) {
					VariantResolution::Found(enum_name, _) => {
						constraints.push(eq_constraint(subject_ty, Type::Enum(enum_name)).at(pattern.range));
					}
					VariantResolution::Ambiguous => {
						// error already reported
					}
					VariantResolution::NotFound => {
						self.add_value_binding(
							ident.name.clone(),
							Scheme::Forall(vec![], subject_ty),
							ident.range,
						);
					}
				}
			}

			PatternKind::Constructor(name, args) => {
				match self.resolve_variant_pattern(name, &subject_ty, /* nullary_only */ false) {
					VariantResolution::Found(enum_name, params) => {
						constraints.push(eq_constraint(subject_ty, Type::Enum(enum_name)).at(pattern.range));

						if args.len() != params.len() {
							self.error(
								pattern.range,
								ParamCountMismatch {
									expected: params.len(),
									found: args.len(),
								},
							);
							return;
						}

						for (arg, param_ty) in args.iter_mut().zip(params.into_iter()) {
							self.constrain_pattern(arg, param_ty, constraints);
						}
					}
					VariantResolution::Ambiguous => {
						// error already reported
					}
					VariantResolution::NotFound => {
						self.error(
							name.range,
							NameNotBound {
								name: name.name.clone(),
							},
						);
					}
				}
			}

			PatternKind::Tuple(entries) => {
				let mut entry_types = Vec::new();
				for _ in entries.iter() {
					entry_types.push(self.new_type_var());
				}
				constraints
					.push(eq_constraint(subject_ty, Type::Tuple(entry_types.clone())).at(pattern.range));
				for (entry, entry_ty) in entries.iter_mut().zip(entry_types.into_iter()) {
					self.constrain_pattern(entry, entry_ty, constraints);
				}
			}

			PatternKind::Record(fields) => {
				// Use PartialRecord per field so the subject can be a record with
				// at least these fields.
				for (field_name, field_pattern) in fields.iter_mut() {
					let field_ty = self.new_type_var();
					constraints.push(
						eq_constraint(
							subject_ty.clone(),
							Type::PartialRecord(field_name.name.clone(), field_ty.clone().into()),
						)
						.at(field_pattern.range),
					);
					self.constrain_pattern(field_pattern, field_ty, constraints);
				}
			}

			PatternKind::Interpolation(_) => {
				// TODO: interpolation patterns
			}
		}
	}

	fn check_when_exhaustive(&mut self, subject_ty: &Type, cases: &[CaseNode], range: Range) {
		let required: Vec<String> = match subject_ty {
			Type::Bool => vec!["true".into(), "false".into()],
			Type::Enum(name) => match self.enum_defs.get(name) {
				Some(variants) => variants.iter().map(|(n, _)| n.clone()).collect(),
				None => return,
			},
			// Other subject types are an "open universe" (e.g. int, string,
			// records, tuples) — exhaustiveness in that case relies entirely
			// on having a catch-all, which we detect inline below.
			_ => Vec::new(),
		};

		let mut covered = std::collections::HashSet::new();

		for case in cases {
			match &case.pattern.kind {
				PatternKind::Underscore => return,

				PatternKind::Identifier(ident) => {
					// A bare ident either names a nullary variant of the subject enum
					// (covers just that variant) or is a binding (catch-all).
					let is_nullary_variant = match subject_ty {
						Type::Enum(enum_name) => self
							.find_variant_in_enum(enum_name, &ident.name)
							.map_or(false, |p| p.is_empty()),
						_ => false,
					};
					if is_nullary_variant {
						covered.insert(ident.name.clone());
					} else {
						return;
					}
				}

				PatternKind::Constructor(name, args) => {
					// Only count the variant as fully covered if every arg is itself a
					// catch-all sub-pattern. A literal or nested constructor pulls in
					// just a slice of the value space, so we conservatively skip.
					let all_catch = args.iter().all(|arg| match &arg.kind {
						PatternKind::Underscore => true,
						PatternKind::Identifier(ident) => {
							// Treat as binding (catch-all) unless we know it's a nullary
							// variant somewhere. Conservative — counts variants from any
							// enum as non-catch-all.
							self
								.find_variant_globally(&ident.name)
								.iter()
								.all(|(_, p)| !p.is_empty())
						}
						_ => false,
					});
					if all_catch {
						covered.insert(name.name.clone());
					}
				}

				PatternKind::Literal(lit) => {
					if matches!(subject_ty, Type::Bool) {
						if let LiteralKind::Bool(b) = &lit.kind {
							covered.insert(if *b { "true".into() } else { "false".into() });
						}
					}
				}

				_ => {}
			}
		}

		let missing: Vec<String> = required
			.into_iter()
			.filter(|v| !covered.contains(v))
			.collect();

		if !missing.is_empty() {
			self.error(range, WhenNotExhaustive { missing });
		}
	}

	fn find_variant_in_enum(&self, enum_name: &str, variant_name: &str) -> Option<Vec<Type>> {
		self
			.enum_defs
			.get(enum_name)?
			.iter()
			.find(|(n, _)| n == variant_name)
			.map(|(_, params)| params.clone())
	}

	fn find_variant_globally(&self, name: &str) -> Vec<(String, Vec<Type>)> {
		let mut results = Vec::new();
		for (enum_name, variants) in &self.enum_defs {
			for (variant_name, params) in variants {
				if variant_name == name {
					results.push((enum_name.clone(), params.clone()));
				}
			}
		}
		results
	}

	// Resolve a variant name in pattern position. Uses the subject type to
	// disambiguate when known; otherwise falls back to a global lookup and
	// reports an ambiguity error if more than one enum matches.
	fn resolve_variant_pattern(
		&mut self,
		name: &IdentifierNode,
		subject_ty: &Type,
		nullary_only: bool,
	) -> VariantResolution {
		match subject_ty {
			Type::Enum(enum_name) => match self.find_variant_in_enum(enum_name, &name.name) {
				Some(params) => {
					if nullary_only && !params.is_empty() {
						return VariantResolution::NotFound;
					}
					VariantResolution::Found(enum_name.clone(), params)
				}
				None => VariantResolution::NotFound,
			},

			_ => {
				let mut candidates = self.find_variant_globally(&name.name);
				if nullary_only {
					candidates.retain(|(_, p)| p.is_empty());
				}
				match candidates.len() {
					0 => VariantResolution::NotFound,
					1 => {
						let (enum_name, params) = candidates.into_iter().next().unwrap();
						VariantResolution::Found(enum_name, params)
					}
					_ => {
						let mut enums: Vec<String> = candidates
							.into_iter()
							.map(|(n, _)| {
								// Display the bare enum name; the qualifier is
								// internal-only and would be redundant when both
								// candidates share the same defining module.
								n.rsplit_once('.').map(|(_, b)| b.to_string()).unwrap_or(n)
							})
							.collect();
						enums.sort();
						self.error(
							name.range,
							AmbiguousVariant {
								name: name.name.clone(),
								enums,
							},
						);
						VariantResolution::Ambiguous
					}
				}
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
						reason.range,
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
				// tuples can only be unified if they have the same number of elements, with
				// the same types, in the same order. That is: type `(int, string)` is not
				// equivalent to `(string, int)`.

				if element_types_1.len() != element_types_2.len() {
					self.error(
						reason.range,
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

			Eq(Type::Tuple(element_types), Type::PartialTuple(index, element_type), reason)
			| Eq(Type::PartialTuple(index, element_type), Type::Tuple(element_types), reason) => {
				// tuples and partial tuples can be unified in a manner that's less strict than
				// unifying two tuples: the tuple must only match the partial tuple at the given index

				if index > &element_types.len() {
					self.error(
						reason.range,
						TupleIndexNotPresent {
							index: *index,
							ty: Type::Tuple(element_types.clone()),
						},
					);

					return Substitution::empty();
				}

				let mut constraints = Vec::with_capacity(1);

				// tuple at index should have same type as this whole expr
				constraints.push(eq_constraint(
					element_types[*index].clone(),
					*element_type.clone(),
				));

				self.unify(&constraints)
			}

			Eq(Type::Record(field_types_1), Type::Record(field_types_2), reason) => {
				// records can be unified if they have all the same field names, and the same
				// types at each field name, but order does not matter
				// NOTE: actually, maybe it's fine if {name: string, age: int} is given when
				// we wanted {age: int} ? todo later?

				// add some new constraints to unify field types:
				let mut constraints = Vec::with_capacity(field_types_2.len());

				let expected_fields: HashMap<&String, usize> =
					HashMap::from_iter(field_types_2.iter().enumerate().map(|(i, (k, _))| (k, i)));
				let actual_fields: HashMap<&String, usize> =
					HashMap::from_iter(field_types_1.iter().enumerate().map(|(i, (k, _))| (k, i)));

				let mut errors = false;

				for (field_name, expected_index) in expected_fields {
					if let Some(actual_index) = actual_fields.get(field_name) {
						constraints.push(eq_constraint(
							field_types_1[*actual_index].1.clone(),
							field_types_2[expected_index].1.clone(),
						))
					} else {
						errors = true;

						self.error(
							reason.range,
							RecordFieldNotPresent {
								field: field_name.clone(),
								ty: Type::Record(field_types_1.clone()),
							},
						)
					}
				}

				if errors {
					return Substitution::empty();
				}

				self.unify(&constraints)
			}

			Eq(Type::Record(field_types), Type::PartialRecord(field_name, field_type), reason)
			| Eq(Type::PartialRecord(field_name, field_type), Type::Record(field_types), reason) => {
				// records and partial records can be unified as long as the record has the field
				// referenced by the partial record and the fields have the same type

				let fields_with_indices: HashMap<&String, usize> =
					HashMap::from_iter(field_types.iter().enumerate().map(|(i, (k, _))| (k, i)));

				if let Some(field_index) = fields_with_indices.get(field_name) {
					let mut constraints = Vec::with_capacity(1);

					// record at field should have same type as the one in the partial record
					constraints.push(eq_constraint(
						field_types[*field_index].1.clone(),
						*field_type.clone(),
					));

					return self.unify(&constraints);
				} else {
					self.error(
						reason.range,
						RecordFieldNotPresent {
							field: field_name.clone(),
							ty: Type::Record(field_types.clone()),
						},
					);

					return Substitution::empty();
				}
			}

			Eq(Type::Enum(name_1), Type::Enum(name_2), _) if name_1 == name_2 => Substitution::empty(),

			Eq(Type::Var(n), t, reason) | Eq(t, Type::Var(n), reason) => match t {
				Type::Var(n2) if n == n2 => Substitution::empty(),
				Type::Var(_) => Substitution::with_entry(*n, t.clone()),
				other => {
					if other.contains_var(*n) {
						self.error(reason.range, RecursiveUnification { ty: other.clone() });
						return Substitution::empty();
					}

					Substitution::with_entry(*n, t.clone())
				}
			},

			Eq(a, b, reason) => {
				self.error(
					reason.range,
					TypeMismatch {
						expected: b.clone(),
						found: a.clone(),
					},
				);

				Substitution::empty()
			}

			_ => unreachable!("should only have eq constraints in here"),
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

			_ => unreachable!("should have a gen first"),
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
			self.fill_in_placeholder(&mut definition.ty, subst);

			// The def itself is a statement with no type:
			definition.ty = Type::Nothing;

			match &mut definition.kind {
				DefinitionKind::Expr(expr) => {
					// But when defining exprs, we must annotate within the def value:
					self.annotate_expr(expr, subst);
				}
				_ => { /* nothing to do for other def kinds */ }
			}
		}
	}

	fn annotate_expr(&mut self, expr: &mut ExprNode, subst: &Substitution) {
		self.fill_in_placeholder(&mut expr.ty, subst);
		let expr_range = expr.range;

		match &mut expr.kind {
			ExprKind::Let(LetNode { value, .. }) => {
				self.annotate_expr(value, subst);
			}

			ExprKind::Fun(FunNode { params, body, .. }) => {
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

			ExprKind::Record(fields) => {
				for (_, field_value) in fields {
					self.annotate_expr(field_value, subst);
				}
			}

			ExprKind::Interpolation(parts) => {
				for part in parts {
					self.annotate_expr(part, subst);
				}
			}

			ExprKind::ElementAccess { receiver, .. } => {
				self.annotate_expr(receiver, subst);
			}

			ExprKind::FieldAccess { receiver, .. } => {
				self.annotate_expr(receiver, subst);
			}

			ExprKind::BinaryOperation { left, right, .. } => {
				self.annotate_expr(left, subst);
				self.annotate_expr(right, subst);
			}

			ExprKind::When(WhenNode { subject, cases, .. }) => {
				self.annotate_expr(subject, subst);
				let subject_ty = subject.ty.clone();
				self.check_when_exhaustive(&subject_ty, cases, expr_range);
				for case in cases.iter_mut() {
					for body_expr in case.body.iter_mut() {
						self.annotate_expr(body_expr, subst);
					}
				}
			}

			ExprKind::If(IfNode { subject, body, .. }) => {
				self.annotate_expr(subject, subst);
				for body_expr in body.iter_mut() {
					self.annotate_expr(body_expr, subst);
				}
			}

			ExprKind::While(WhileNode { subject, body, .. }) => {
				self.annotate_expr(subject, subst);
				for body_expr in body.iter_mut() {
					self.annotate_expr(body_expr, subst);
				}
			}

			ExprKind::Grouping(inner) => {
				self.annotate_expr(inner, subst);
			}

			ExprKind::Identifier(_) => {
				// nothing to annotate!
			}

			ExprKind::Literal(_) => {
				// nothing to annotate!
			}

			ExprKind::Regex(_) => {
				// nothing to annotate?
			}

			other => {
				if cfg!(debug_assertions) {
					todo!("analyze expr kind: {:?}", other);
				}
			}
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

	// Produce a fresh copy of `ty` with every free Var consistently replaced
	// by a new type var. Used when reaching across modules to a value whose
	// type may contain free vars (i.e. is polymorphic).
	fn instantiate(&mut self, ty: &Type) -> Type {
		let mut mapping: HashMap<usize, Type> = HashMap::new();
		self.instantiate_with(ty, &mut mapping)
	}

	fn instantiate_with(&mut self, ty: &Type, mapping: &mut HashMap<usize, Type>) -> Type {
		match ty {
			Type::Var(n) => {
				if let Some(replacement) = mapping.get(n) {
					replacement.clone()
				} else {
					let fresh = self.new_type_var();
					mapping.insert(*n, fresh.clone());
					fresh
				}
			}
			Type::Fun(params, ret) => Type::Fun(
				params
					.iter()
					.map(|t| self.instantiate_with(t, mapping))
					.collect(),
				Box::new(self.instantiate_with(ret, mapping)),
			),
			Type::Tuple(elems) => Type::Tuple(
				elems
					.iter()
					.map(|t| self.instantiate_with(t, mapping))
					.collect(),
			),
			Type::Record(fields) => Type::Record(
				fields
					.iter()
					.map(|(n, t)| (n.clone(), self.instantiate_with(t, mapping)))
					.collect(),
			),
			Type::Enum(..)
			| Type::Bool
			| Type::Int
			| Type::Float
			| Type::String
			| Type::Regex
			| Type::Unknown
			| Type::Nothing => ty.clone(),
			Type::PartialRecord(name, inner) => Type::PartialRecord(
				name.clone(),
				Box::new(self.instantiate_with(inner, mapping)),
			),
			Type::PartialTuple(index, inner) => {
				Type::PartialTuple(*index, Box::new(self.instantiate_with(inner, mapping)))
			}
		}
	}
}
