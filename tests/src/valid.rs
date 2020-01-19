use crate::*;
#[cfg(test)]
use insta::assert_snapshot;
#[cfg(test)]
use pluma_compiler::parser::Parser;
#[cfg(test)]
use pluma_compiler::tokenizer::Tokenizer;

test_valid!(empty, "");

test_valid!(identifier, "hello");

test_valid!(qualified_identifier, "qual:hello");

test_valid!(number, "47");

test_valid!(string, "\"hello\"");

test_valid!(string_with_interpolation, "\"hello $(name)!\"");

test_valid!(
  string_with_nested_interpolation,
  "\"hello $(\"aa $(name) bb\")!\""
);

test_valid!(assignment_constant, "let x = 47");

test_valid!(assignment_variable, "let x := 47");

test_valid!(reassignment_variable, "x := 47");

test_valid!(err_incomplete_assignment, "let");

test_valid!(err_incomplete_assignment_2, "let x");

test_valid!(err_incomplete_assignment_3, "let x\nlet y = 3");

test_valid!(import_module, "use something");

test_valid!(import_module_with_alias, "use something as alias");

test_valid!(
  import_two_modules,
  "use something
use path/to/module
let x = 47"
);

test_valid!(call, "func()");

test_valid!(call_with_args, "func(1, \"two\", three)");

test_valid!(
  non_call_multiple_lines,
  "thing
(just, a, tuple)"
);

test_valid!(
  non_call_block_after_identifier,
  "x
{}"
);

test_valid!(
  non_call_block_after_assignment,
  "let x = 2
{}"
);

test_valid!(method_def_no_params, "def someMethod() = {}");

test_valid!(method_def_no_params_with_body, "def someMethod() = { 47 }");

test_valid!(
  method_def_no_params_multiline,
  "def someMethod() = {
  47
}"
);

test_valid!(method_def_with_params, "def someMethod() = { a, b => 47 }");
