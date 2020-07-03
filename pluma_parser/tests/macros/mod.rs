#[macro_export]
macro_rules! test_parse_success {
  ($($name:ident: $source:literal,)*) => {
    $(
        #[test]
        fn $name() {
            use insta::assert_snapshot;
            use pluma_parser::*;
            use std::collections::HashMap;

            let replaced = $source.replace("\n    |", "\n");
            let source = replaced.trim();
            let source_copy = source.clone();
            let bytes = Vec::from(source);
            let tokenizer = Tokenizer::from_source(&bytes, false);
            let mut parser = Parser::new(&bytes, tokenizer, false);
            let (ast, _imports, _, errors) = parser.parse_module();

            if !errors.is_empty() {
              panic!("parse errors: {:#?}", errors);
            }

            let file_name = format!("{}", stringify!($name));

            let formatted = format!("
=== Source ===
{}

=== Comments ===
{:#?}

=== AST ===
{:#?}
", source_copy, HashMap::<(), ()>::new(), ast);

            assert_snapshot!(file_name, formatted, &source_copy);
        }
    )*
  }
}
