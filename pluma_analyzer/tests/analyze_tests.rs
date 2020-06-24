#[macro_use]
mod macros;

test_analyze! {
  undefined_identifier_simple (false): r#"
    |wat
  "#,

  defined_identifier_simple (true): r#"
    |let fine = 47
    |
    |fine
  "#,

  intrinsic_def_binary_op_valid (true): r#"
    |intrinsic_type Int
    |intrinsic_def Int + Int -> Int
    |
    |let result = 3 + 4
  "#,

  intrinsic_def_binary_op_invalid (false): r#"
    |intrinsic_type Int
    |intrinsic_def Int + Int -> Int
    |
    |let result = 3 + "yikes"
  "#,
}
