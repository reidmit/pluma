#[macro_export]
macro_rules! test_analyze {
  ($($name:ident ($no_errors:literal): $source:literal,)*) => {
    $(
        #[test]
        fn $name() {
            use insta::assert_snapshot;
            use pluma_analyzer::*;
            use pluma_parser::*;
            use pluma_visitor::Traverse;

            let replaced = $source.replace("\n    |", "\n");
            let source = replaced.trim();
            let source_copy = source.clone();
            let bytes = Vec::from(source);
            let tokenizer = Tokenizer::from_source(&bytes);
            let mut parser = Parser::new(&bytes, tokenizer);
            let (mut ast, _imports, errors) = parser.parse_module();

            if !errors.is_empty() {
              panic!("parse errors: {:#?}", errors);
            }

            let mut scope = Scope::new();
            let mut diagnostics = Vec::new();

            scope.enter();

            let mut type_collector = TypeCollector::new(&mut scope);
            ast.traverse(&mut type_collector);
            diagnostics.append(&mut type_collector.diagnostics);

            let mut analyzer = Analyzer::new(&mut scope);
            ast.traverse(&mut analyzer);
            diagnostics.append(&mut analyzer.diagnostics);

            let file_name = format!("{}", stringify!($name));
            let formatted;

            if $no_errors {
              if !diagnostics.is_empty() {
                panic!("expected no analysis errors, but got: {:#?}", diagnostics);
              }

              formatted = format!("
=== Source ===
{}

=== Top-level scope ===
{:#?}
", source_copy, scope);
            } else {
            if diagnostics.is_empty() {
              panic!("expected analysis errors, but found none");
            }


            formatted = format!("
=== Source ===
{}

=== Diagnostics ===
{:#?}
", source_copy, diagnostics);
            }

            assert_snapshot!(file_name, formatted, &source_copy);
        }
    )*
  }
}
