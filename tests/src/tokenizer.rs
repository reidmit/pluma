#[cfg(test)]
mod tests {
  use pluma_compiler::tokenizer::Tokenizer;
  use crate::assert_tokens_snapshot;
  use insta::assert_snapshot;

  assert_tokens_snapshot!(
    empty,
    ""
  );

  assert_tokens_snapshot!(
    identifers,
    "hello world"
  );

  assert_tokens_snapshot!(
    numbers,
    "hello 1 47 wow"
  );

  assert_tokens_snapshot!(
    binary_numbers,
    "0b101 0b00 0b1 0B0 0b00100"
  );

  assert_tokens_snapshot!(
    hex_numbers,
    "0x101 0x0 0xfacade 0X47"
  );

  assert_tokens_snapshot!(
    octal_numbers,
    "0o101 0o0 0o47 0O47"
  );

  assert_tokens_snapshot!(
    comment_before,
    "# comment \nok"
  );

  assert_tokens_snapshot!(
    comment_same_line,
    "ok #comment"
  );

  assert_tokens_snapshot!(
    comment_after,
    "ok \n\n#comment"
  );

  assert_tokens_snapshot!(
    symbols,
    "{ . } ( , ) : [ :: ] := = => ->"
  );

  assert_tokens_snapshot!(
    unexpected,
    "(@$@)"
  );

  assert_tokens_snapshot!(
    strings_without_interpolations,
    "\"hello\" \"\" \"world\""
  );

  assert_tokens_snapshot!(
    strings_with_interpolations,
    "\"hello $(name)!\" nice \"$(str)\""
  );

  assert_tokens_snapshot!(
    strings_with_nested_interpolations,
    "\"hello $(name \"inner $(o)\" wow)!\""
  );

  assert_tokens_snapshot!(
    import,
    "use path/to/module"
  );

  assert_tokens_snapshot!(
    import_multiple,
    "use path/to/module\nuse another-module"
  );

  assert_tokens_snapshot!(
    import_with_let,
    "use thing\nlet x = 47"
  );
}
