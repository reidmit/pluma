use super::*;
use crate::location::Range;

pub struct UseNode {
	pub range: Range,
	// module path segments; e.g. `use sub/utils` produces [sub, utils].
	// `module_name()` joins them with `/` for the internal name.
	pub path: Vec<IdentifierNode>,
	// optional `as <ident>` alias; if None, the local name is the last
	// path segment.
	pub alias: Option<IdentifierNode>,
}

impl UseNode {
	pub fn module_name(&self) -> String {
		self
			.path
			.iter()
			.map(|p| p.name.clone())
			.collect::<Vec<_>>()
			.join("/")
	}

	pub fn local_name(&self) -> &IdentifierNode {
		self.alias.as_ref().unwrap_or_else(|| {
			self
				.path
				.last()
				.expect("use path must have at least one segment")
		})
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for UseNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &self.alias {
			Some(alias) => write!(
				f,
				"use({:#?}) `{}` as `{}`",
				self.range,
				self.module_name(),
				alias.name
			),
			None => write!(f, "use({:#?}) `{}`", self.range, self.module_name()),
		}
	}
}
