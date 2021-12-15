use ast::*;
use std::collections::HashMap;

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Clone)]
pub struct Binding {
	pub typ: ValueType,
	pub ref_count: usize,
	pub pos: (usize, usize),
	pub kind: BindingKind,
}

#[derive(PartialEq, Clone)]
#[cfg_attr(debug_assertions, derive(Debug))]
pub enum BindingKind {
	Const,
	Let,
	Def,
	Param,
	EnumVariant,
	StructConstructor,
	Field,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct TypeBinding {
	pub ref_count: usize,
	pub pos: (usize, usize),
	pub kind: TypeBindingKind,
	pub methods: HashMap<Vec<String>, ValueType>,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TypeBindingKind {
	Enum,
	Struct { fields: HashMap<String, Binding> },
	Alias,
	Trait { fields: HashMap<String, Binding> },
	IntrinsicType,
}

impl TypeBinding {
	pub fn fields(&self) -> HashMap<String, Binding> {
		match &self.kind {
			TypeBindingKind::Struct { fields } => (*fields).clone(),
			TypeBindingKind::Trait { fields } => (*fields).clone(),
			_ => unreachable!(),
		}
	}

	pub fn field_types(&self) -> HashMap<String, ValueType> {
		let mut field_types = HashMap::new();

		match &self.kind {
			TypeBindingKind::Struct { fields } => {
				for (field_name, field_binding) in fields {
					field_types.insert(field_name.clone(), field_binding.typ.clone());
				}
			}
			TypeBindingKind::Trait { fields } => {
				for (field_name, field_binding) in fields {
					field_types.insert(field_name.clone(), field_binding.typ.clone());
				}
			}
			_ => {}
		}

		field_types
	}
}
