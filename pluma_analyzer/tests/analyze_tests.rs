#[macro_use]
mod macros;

test_analyze! {
  undefined_identifier_simple (false): r#"
    |wat
  "#,

  defined_identifier_simple (true): r#"
    |let fine = 47
    |fine
  "#,
}
