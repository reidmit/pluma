use crate::binding::*;
use crate::expr_type::*;
use std::collections::HashMap;

pub fn get_intrinsic_types() -> HashMap<String, TypeBinding> {
  let mut intrinsics = HashMap::new();

  intrinsics.insert(
    "intrinsic-nothing".into(),
    TypeBinding {
      loc: (0, 0),
      typ: ExprType::Nothing,
    },
  );

  intrinsics.insert(
    "intrinsic-bool".into(),
    TypeBinding {
      loc: (0, 0),
      typ: ExprType::Bool,
    },
  );

  intrinsics.insert(
    "intrinsic-int".into(),
    TypeBinding {
      loc: (0, 0),
      typ: ExprType::Int,
    },
  );

  intrinsics.insert(
    "intrinsic-float".into(),
    TypeBinding {
      loc: (0, 0),
      typ: ExprType::Float,
    },
  );

  intrinsics.insert(
    "intrinsic-string".into(),
    TypeBinding {
      loc: (0, 0),
      typ: ExprType::String,
    },
  );

  intrinsics.insert(
    "intrinsic-regex".into(),
    TypeBinding {
      loc: (0, 0),
      typ: ExprType::Regex,
    },
  );

  intrinsics
}

pub fn get_intrinsic_values() -> HashMap<String, ValueBinding> {
  let mut intrinsics = HashMap::new();

  // def print :: string -> nothing
  intrinsics.insert(
    "intrinsic-print".into(),
    ValueBinding {
      loc: (0, 0),
      ref_count: 0,
      typ: ExprType::Func(vec![ExprType::String], Box::new(ExprType::Nothing)),
    },
  );

  intrinsics
}
