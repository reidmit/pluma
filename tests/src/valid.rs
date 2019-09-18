#[allow(unused_imports)]
use pluma_compiler::tokenizer::Tokenizer;
#[allow(unused_imports)]
use pluma_compiler::parser::Parser;
#[allow(unused_imports)]
use insta::assert_snapshot;

macro_rules! assert_valid_snapshot {
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

assert_valid_snapshot!(
  empty,
  ""
);

assert_valid_snapshot!(
  identifier,
  "hello"
);

assert_valid_snapshot!(
  number,
  "47"
);

assert_valid_snapshot!(
  string,
  "\"hello\""
);

assert_valid_snapshot!(
  string_with_interpolation,
  "\"hello $(name)!\""
);

assert_valid_snapshot!(
  string_with_nested_interpolation,
  "\"hello $(\"aa $(name) bb\")!\""
);

assert_valid_snapshot!(
  assignment_constant,
  "let x = 47"
);

assert_valid_snapshot!(
  assignment_variable,
  "let x := 47"
);

assert_valid_snapshot!(
  reassignment_variable,
  "x := 47"
);

assert_valid_snapshot!(
  err_incomplete_assignment,
  "let"
);

assert_valid_snapshot!(
  err_incomplete_assignment_2,
  "let x"
);

assert_valid_snapshot!(
  err_incomplete_assignment_3,
  "let x\nlet y = 3"
);

assert_valid_snapshot!(
  import_module,
  "use something"
);

assert_valid_snapshot!(
  import_module_with_alias,
  "use something as alias"
);

assert_valid_snapshot!(
  import_two_modules,
  "use something\nuse path/to/module\nlet x = 47"
);

assert_valid_snapshot!(
  call,
  "func()"
);

assert_valid_snapshot!(
  call_with_args,
  "func(1, \"two\", three)"
);

assert_valid_snapshot!(
  non_call_multiple_lines,
  "thing\n(just, a, tuple)"
);
