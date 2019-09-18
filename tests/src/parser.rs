#[cfg(test)]
mod tests {
  use pluma_compiler::tokenizer::Tokenizer;
  use pluma_compiler::parser::Parser;
  use crate::assert_parsed_snapshot;
  use insta::assert_snapshot;

  assert_parsed_snapshot!(
    empty,
    ""
  );

  assert_parsed_snapshot!(
    identifier,
    "hello"
  );

  assert_parsed_snapshot!(
    number,
    "47"
  );

  assert_parsed_snapshot!(
    string,
    "\"hello\""
  );

  assert_parsed_snapshot!(
    string_with_interpolation,
    "\"hello $(name)!\""
  );

  assert_parsed_snapshot!(
    string_with_nested_interpolation,
    "\"hello $(\"aa $(name) bb\")!\""
  );

  assert_parsed_snapshot!(
    assignment_constant,
    "let x = 47"
  );

  assert_parsed_snapshot!(
    assignment_variable,
    "let x := 47"
  );

  assert_parsed_snapshot!(
    reassignment_variable,
    "x := 47"
  );

  assert_parsed_snapshot!(
    err_incomplete_assignment,
    "let"
  );

  assert_parsed_snapshot!(
    err_incomplete_assignment_2,
    "let x"
  );

  assert_parsed_snapshot!(
    err_incomplete_assignment_3,
    "let x\nlet y = 3"
  );

  assert_parsed_snapshot!(
    import_module,
    "use something"
  );

  assert_parsed_snapshot!(
    import_module_with_alias,
    "use something as alias"
  );

  assert_parsed_snapshot!(
    import_two_modules,
    "use something\nuse path/to/module\nlet x = 47"
  );

  assert_parsed_snapshot!(
    call,
    "func()"
  );

  assert_parsed_snapshot!(
    call_with_args,
    "func(1, \"two\", three)"
  );

  assert_parsed_snapshot!(
    non_call_multiple_lines,
    "thing\n(just, a, tuple)"
  );
}
