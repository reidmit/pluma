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

#[macro_export]
macro_rules! assert_tokens_snapshot {
  ($name: ident, $source: literal) => {
    #[test]
    fn $name() {
      let src = $source;
      let v = Vec::from($source);
      let tokens = Tokenizer::from_source(&v).collect_tokens().unwrap();
      let value = format!("{:#?}", tokens);

      assert_snapshot!(stringify!($name), value, src);
    }
  };
}