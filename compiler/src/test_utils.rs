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
      let file_name = format!("tokenize_{}", stringify!($name));

      assert_snapshot!(file_name, value, src);
    }
  };
}

#[macro_export]
macro_rules! assert_parsed_snapshot {
  ($name: ident, $source: literal) => {
    #[test]
    fn $name() {
      let src = $source;
      let v = Vec::from($source);
      let ast = Parser::from_source(&v).parse_module();
      let value = format!("{:#?}", ast);
      let file_name = format!("parse_{}", stringify!($name));

      assert_snapshot!(file_name, value, src);
    }
  };
}