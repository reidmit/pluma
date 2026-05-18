// Builtin tags. The VM owns the implementations (in eval.rs); codegen
// emits CallBuiltin instructions referring to these by tag.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Builtin {
	Print,
	ToString,
	Matches,
	ListLength,
	ListIsEmpty,
	ListReverse,
	ListConcat,
	ListContains,
	ListMap,
	ListFilter,
	ListFold,
	ListEach,
	MathToFloat,
	MathToInt,
	MathSqrt,
	MathAbs,
}
