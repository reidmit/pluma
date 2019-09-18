#[macro_export]
macro_rules! assert_tokens_snapshot {
  ($name: ident, $source: literal) => {
    #[test]
    fn $name() {
      let src = $source;
      let bytes = Vec::from($source);
      let mut tokenizer = Tokenizer::from_source(&bytes);
      let result = tokenizer.collect_tokens();
      let value = format!("{:#?}", result);
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
      let bytes = Vec::from($source);
      let mut tokenizer = Tokenizer::from_source(&bytes);
      let (tokens, _) = tokenizer.collect_tokens().unwrap();
      let ast = Parser::new(&bytes, &tokens).parse_module();
      let value = format!("{:#?}", ast);
      let file_name = format!("parse_{}", stringify!($name));

      assert_snapshot!(file_name, value, src);
    }
  };
}