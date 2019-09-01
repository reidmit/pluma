#[macro_export]
macro_rules! expect_eq {
  ($left:expr, $right:expr) => {{
    match (&$left, &$right) {
      (left_val, right_val) => {
        if !(*left_val == *right_val) {
          panic!(
            r#"expectation failed: `(left == right)`
  left: `{:#?}`,
 right: `{:#?}`"#,
            &*left_val, &*right_val
          )
        }
      }
    }
  }};
}
