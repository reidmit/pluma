#[macro_export]
macro_rules! test_valid {
  ($name: ident, $source: literal) => {
    #[test]
    fn $name() {
      let src = $source;
      let bytes = Vec::from($source);
      let mut tokenizer = Tokenizer::from_source(&bytes);
      let (tokens, comments) = tokenizer.collect_tokens().unwrap();
      let ast = Parser::new(&bytes, &tokens).parse_module();

      let value = format!(
        "\n{}\n{}\n\n{}\n{:#?}\n\n{}\n{:#?}\n\n{}\n{:#?}",
        "=== Source ===",
        src,
        "=== Tokens ===",
        tokens,
        "=== Comments ===",
        comments,
        "=== AST ===",
        ast,
      );

      let file_name = format!("{}", stringify!($name));

      assert_snapshot!(file_name, value, src);
    }
  };
}