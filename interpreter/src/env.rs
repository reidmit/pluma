use crate::value::Value;
use std::collections::HashMap;

// Lexical scope stack. Closures clone the env at creation time — since values
// share strings/lists/records via Rc, that's cheap for typical programs.
#[derive(Clone)]
pub struct Environment<'ast> {
	scopes: Vec<HashMap<String, Value<'ast>>>,
}

impl<'ast> Environment<'ast> {
	pub fn new() -> Self {
		Self {
			scopes: vec![HashMap::new()],
		}
	}

	pub fn enter_scope(&mut self) {
		self.scopes.push(HashMap::new());
	}

	pub fn leave_scope(&mut self) {
		self.scopes.pop();
	}

	pub fn define(&mut self, name: String, value: Value<'ast>) {
		self.scopes
			.last_mut()
			.expect("env has no scope")
			.insert(name, value);
	}

	pub fn lookup(&self, name: &str) -> Option<&Value<'ast>> {
		for scope in self.scopes.iter().rev() {
			if let Some(v) = scope.get(name) {
				return Some(v);
			}
		}
		None
	}
}
