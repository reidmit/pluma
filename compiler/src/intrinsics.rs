use crate::binding::*;
use crate::expr_type::*;
use std::collections::HashMap;

pub fn get_intrinsic_values() -> HashMap<String, ValueBinding> {
  let mut intrinsics = HashMap::new();

  // def print :: string -> nothing
  intrinsics.insert(
    "intrinsic-print".into(),
    ValueBinding {
      span: (0, 0),
      ref_count: 0,
      typ: ExprType::Func(vec![ExprType::String], Box::new(ExprType::Nothing)),
    },
  );

  intrinsics
}
