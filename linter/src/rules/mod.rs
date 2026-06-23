//! The lint catalog. One module per rule; each exports a unit struct that
//! implements [`crate::Rule`]. Register new rules in `crate::rules()`.

mod bind_then_return;
mod identical_branches;
mod if_chain_as_when;
mod if_returns_bool;
mod prefer_using_block;
mod redundant_bool_comparison;
mod redundant_bool_operand;
mod redundant_lambda;
mod redundant_let_underscore;
mod redundant_try_underscore;
mod redundant_using_prefix;
mod when_as_if;

pub use bind_then_return::BindThenReturn;
pub use identical_branches::IdenticalBranches;
pub use if_chain_as_when::IfChainAsWhen;
pub use if_returns_bool::IfReturnsBool;
pub use prefer_using_block::PreferUsingBlock;
pub use redundant_bool_comparison::RedundantBoolComparison;
pub use redundant_bool_operand::RedundantBoolOperand;
pub use redundant_lambda::RedundantLambda;
pub use redundant_let_underscore::RedundantLetUnderscore;
pub use redundant_try_underscore::RedundantTryUnderscore;
pub use redundant_using_prefix::RedundantUsingPrefix;
pub use when_as_if::WhenAsIf;
