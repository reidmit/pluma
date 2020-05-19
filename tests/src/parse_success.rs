use insta::assert_snapshot;
use pluma_compiler::parser::Parser;
use pluma_compiler::tokenizer::Tokenizer;

test_parse_success! {
  empty: r#"
  "#,

  number: r#"
    |47
  "#,

  string: r#"
    |"wow"
  "#,

  string_multiple_lines: r#"
    |"wow
    |this
    |   is
    |cool!"
  "#,

  identifier: r#"
    |cool
  "#,

  list_empty: r#"
    |[]
  "#,

  list_of_numbers: r#"
    |[1, 2, 3]
  "#,

  list_of_strings: r#"
    |["hey", "wow", "cool", "multi-
    |
    |line"]
  "#,

  list_of_expressions: r#"
    |[1, callThing(), repeat 3 times "wow"]
  "#,

  dict_empty: r#"
    |[:]
  "#,

  def_one_part_empty_arg: r#"
    |def hello () {
    |  a => "wow!"
    |}
  "#,

  def_one_part_one_arg: r#"
    |def hello String {
    |  a => "wow!"
    |}
  "#,

  def_one_part_two_args: r#"
    |def hello (String, Int) {
    |  a, b => "wow!"
    |}
  "#,

  def_two_part_empty_arg: r#"
    |def hello () world () {
    |  a, b => "wow!"
    |}
  "#,

  def_two_part_multiple_args: r#"
    |def hello (A, B) world C {
    |  a, b, c => "wow!"
    |}
  "#,

  def_receiver_one_part_one_arg: r#"
    |def Person . greet String {
    |  a => "wow!"
    |}
  "#,

  def_receiver_two_parts_multiple_args: r#"
    |def Person . hello (String, Int) world () {
    |  a, b, c => "wow!"
    |}
  "#,

  def_return_type: r#"
    |def hello () -> String {
    |  a => "wow!"
    |}
  "#,

  def_func_arg: r#"
    |def hello { A -> B } -> String {
    |  a => "wow!"
    |}
  "#,

  def_func_taking_tuple_arg: r#"
    |def hello { (A, B) -> C } {
    |  x => x
    |}
  "#,

  def_func_return_type: r#"
    |def hello () -> { A -> B } {
    |  { x => y }
    |}
  "#,

  call_empty_arg: r#"
    |func()
  "#,

  call_number_arg: r#"
    |func 1
  "#,

  call_tuple_arg: r#"
    |func (1, "wow")
  "#,

  call_multiple_parts: r#"
    |multiply 1 by 2
  "#,

  call_with_parens_around_single_args: r#"
    |multiply(1)by(2)
  "#,

  call_block_arg: r#"
    |do { print "hey" }
  "#,

  call_spanning_multiple_lines: r#"
    |if thing then {
    |  print "yep!"
    |} else {
    |  print "nope!"
    |}
  "#,

  chain_one_line: r#"
    |"hello" . f1 () . f2 "wow" .f3(47)
  "#,

  chain_field_access: r#"
    |x . field1 . field2 . field3
  "#,

  chain_call_multiple_parts: r#"
    |"hello" . replace "x" with "y"
  "#,

  chain_across_lines: r#"
    |"hello"
    |  . f1
    |  .f2
  "#,

  chain_calls_across_lines: r#"
    |"hello"
    |  . f1 1
    |  .f2 2
  "#,

  private_defs: r#"
    |def isPublic() {}
    |
    |private
    |
    |def isPrivate() {}
  "#,
}
