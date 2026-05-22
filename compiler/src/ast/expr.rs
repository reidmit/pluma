use super::*;
use crate::location::*;
use crate::types::*;

#[derive(Clone)]
pub struct ExprNode {
	pub ty: Type,
	pub kind: ExprKind,
	pub range: Range,
	// `Some(cell)` when this expression's VALUE is a typeclass method
	// dispatch (e.g. the `numeric.add` FieldAccess, a `BinaryOperation`
	// `a + b`, or a `UnaryOperation` `-a`). Cell is shared with the
	// matching `Class` constraint; discharge fills it in.
	pub trait_dispatch: Option<DispatchCell>,
	// Set on an Identifier ExprNode that references a polymorphic
	// constrained value (e.g. `double` whose scheme is
	// `forall a. Numeric a => a -> a`). Holds the dispatch cells the
	// surrounding Call should consume into its `dict_args`. Each cell
	// is filled in by Gen/Inst processing in `unify`.
	pub dispatch_sink: Option<DispatchSink>,
}

#[derive(Clone)]
pub enum ExprKind {
	BinaryOperation {
		op: OperatorNode,
		left: Box<ExprNode>,
		right: Box<ExprNode>,
	},

	UnaryOperation {
		op: Operator,
		right: Box<ExprNode>,
	},

	/// e.g. `someTuple.0` or `(0, 1, 2).1`
	ElementAccess {
		receiver: Box<ExprNode>,
		index: usize,
	},

	/// e.g. `someRecord.field` or `{name: "reid"}.name`
	FieldAccess {
		receiver: Box<ExprNode>,
		field: IdentifierNode,
	},

	/// A namespace path like `math.add`, `numeric.add`, `EnumName.variant`,
	/// or `module.EnumName.variant`. The parser produces these as nested
	/// `FieldAccess` exprs; the analyzer rewrites them once it identifies
	/// the receiver as a namespace (imported module / trait / enum-type)
	/// rather than a value. The path has 2 or 3 segments. The expr's `ty`
	/// plus `trait_dispatch` / `dispatch_sink` carry the resolved info that
	/// codegen needs to choose between global-load, trait-dispatch-load,
	/// and variant-constructor lowerings.
	NamespaceAccess(Vec<IdentifierNode>),

	Fun(FunNode),
	Call(CallNode),
	EmptyTuple,
	Grouping(Box<ExprNode>),
	Identifier(IdentifierNode),
	Interpolation(Vec<ExprNode>),
	Let(LetNode),
	Literal(LiteralNode),
	Record(Vec<(IdentifierNode, ExprNode)>),
	Tuple(Vec<ExprNode>),
	Regex(RegexNode),
	/// `try Pattern = Expr ; rest...`. The analyzer peeks the RHS's
	/// inferred head constructor and rewrites this into a
	/// `<carrier>.then` call wrapping `rest` as a continuation. Always
	/// produced by `parse_body_expressions` — never appears at
	/// expression position in source.
	Try(TryNode),

	// the below are not fully implemented yet!
	List(Vec<ExprNode>),
	If(IfNode),
	When(WhenNode),
	While(WhileNode),
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ExprNode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct(&format!("expr({:#?}) :: {}", self.range, self.ty))
			.field("kind", &self.kind)
			.finish()
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ExprKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		use ExprKind::*;

		match &self {
			BinaryOperation { op, left, right } => f
				.debug_struct(&format!("binary {:#?}", op))
				.field("left", left)
				.field("right", right)
				.finish(),

			UnaryOperation { op, right } => {
				write!(f, "unary {} {:#?}", op, right)
			}

			ElementAccess { receiver, index } => f
				.debug_struct("element-access")
				.field("receiver", receiver)
				.field("index", index)
				.finish(),

			FieldAccess { receiver, field } => f
				.debug_struct("field-access")
				.field("receiver", receiver)
				.field("field", field)
				.finish(),

			NamespaceAccess(path) => {
				let joined = path
					.iter()
					.map(|p| p.name.clone())
					.collect::<Vec<_>>()
					.join(".");
				write!(f, "namespace-access `{}`", joined)
			}

			Fun(fun) => {
				write!(f, "{:#?}", fun)
			}

			Call(call) => {
				write!(f, "{:#?}", call)
			}

			EmptyTuple => {
				write!(f, "()")
			}

			Grouping(inner) => {
				write!(f, "{:#?}", inner)
			}

			Identifier(ident) => {
				write!(f, "{:#?}", ident)
			}

			If(if_node) => {
				write!(f, "{:#?}", if_node)
			}

			Interpolation(parts) => {
				write!(f, "interpolation {:#?}", parts)
			}

			Let(let_node) => {
				write!(f, "{:#?}", let_node)
			}

			List(elements) => {
				write!(f, "{:#?}", elements)
			}

			Literal(literal) => {
				write!(f, "{:#?}", literal)
			}

			Record(fields) => {
				write!(f, "record {:#?}", fields)
			}

			Regex(regex) => {
				write!(f, "{:#?}", regex)
			}

			Tuple(entries) => {
				write!(f, "tuple {:#?}", entries)
			}

			Try(try_node) => {
				write!(f, "{:#?}", try_node)
			}

			When(when_node) => {
				write!(f, "{:#?}", when_node)
			}

			While(while_node) => {
				write!(f, "{:#?}", while_node)
			}
		}
	}
}
