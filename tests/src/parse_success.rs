use insta::assert_snapshot;
use pluma_compiler::parser::Parser;
use pluma_compiler::tokenizer::Tokenizer;

test_parse_success! {
  number: r#"
    |47
  "#,

  string: r#"
    |"wow"
  "#,

  identifier: r#"
    |cool
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
}
