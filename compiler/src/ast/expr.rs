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
	/// `defer Expr` — schedules `Expr` to run when the enclosing function
	/// body exits (by any path: normal return or `try`-failure propagation).
	/// A body-statement form like `let`; its own value is `nothing`. Codegen
	/// lowers it to a zero-arg cleanup thunk pushed onto the frame's cleanup
	/// stack, which the VM walks LIFO at `Return`.
	Defer(Box<ExprNode>),
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

	/// `built-in "tag"`. Legal only as the immediate RHS of a
	/// type-annotated top-level def. The tag is resolved against a
	/// codegen-side table mapping strings to `Builtin` enum variants;
	/// the def's global slot is populated directly with the looked-up
	/// `Value::Builtin`, skipping the thunk path.
	Builtin(String),

	// the below are not fully implemented yet!
	// e.g. `[1, ...xs, 2]`. A plain element is `ListItem::Item`; a `...e`
	// spread (which must itself be a `list`) is `ListItem::Spread`. Spreads
	// may appear at any position, any number of times.
	List(Vec<ListItem>),
	If(IfNode),
	When(WhenNode),
	While(WhileNode),

	/// `scope (as s)? { body }` / `manual scope as s { body }` — structured
	/// concurrency (ASYNC.md Phase 4). The analyzer rewrites this into a call
	/// to the hidden `task.scope-new` kernel builtin, so it never survives to
	/// codegen.
	Scope(ScopeNode),
}

/// One entry in a list literal: either a single element or a spliced
/// sub-list. Both carry an `ExprNode`; only the typing and lowering differ.
#[derive(Clone)]
pub enum ListItem {
	/// A single element: `x` in `[x]`.
	Item(ExprNode),
	/// A spliced sub-list: `...xs` in `[...xs]`. The expr must be a `list`.
	Spread(ExprNode),
}

impl ListItem {
	pub fn expr(&self) -> &ExprNode {
		match self {
			ListItem::Item(e) | ListItem::Spread(e) => e,
		}
	}

	pub fn expr_mut(&mut self) -> &mut ExprNode {
		match self {
			ListItem::Item(e) | ListItem::Spread(e) => e,
		}
	}

	pub fn is_spread(&self) -> bool {
		matches!(self, ListItem::Spread(_))
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for ListItem {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		// `Item` forwards to the inner expr so spread-free list snapshots
		// render exactly as they did before this variant existed.
		match self {
			ListItem::Item(e) => write!(f, "{:#?}", e),
			ListItem::Spread(e) => write!(f, "...{:#?}", e),
		}
	}
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

			Defer(inner) => {
				write!(f, "defer {:#?}", inner)
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

			Builtin(tag) => {
				write!(f, "built-in {:?}", tag)
			}

			When(when_node) => {
				write!(f, "{:#?}", when_node)
			}

			While(while_node) => {
				write!(f, "{:#?}", while_node)
			}

			Scope(scope_node) => {
				write!(f, "{:#?}", scope_node)
			}
		}
	}
}
