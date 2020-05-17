#[macro_export]
macro_rules! test_parse_success {
  ($($name:ident: $source:literal,)*) => {
    $(
        #[test]
        fn $name() {
            let replaced = $source.replace("\n    |", "\n");
            let source = replaced.trim();
            let source_copy = source.clone();
            let bytes = Vec::from(source);
            let mut tokenizer = Tokenizer::from_source(&bytes);
            let (tokens, comments, errors) = tokenizer.collect_tokens();

            if !errors.is_empty() {
              panic!("tokenize errors: {:#?}", errors);
            }

            let mut parser = Parser::new(&bytes, &tokens);
            let (ast, _imports, errors) = parser.parse_module();

            if !errors.is_empty() {
              panic!("parse errors: {:#?}", errors);
            }

            let file_name = format!("{}", stringify!($name));

            let formatted = format!("
=== Source ===
{}

=== Tokens ===
{:#?}

=== Comments ===
{:#?}

=== AST ===
{:#?}
", source_copy, tokens, comments, ast);

            assert_snapshot!(file_name, formatted, &source_copy);
        }
    )*
  }
}
