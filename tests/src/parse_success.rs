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

  string_escape_quote: r#"
    |"line 1\"line 2"
  "#,

  string_escape_backslash: r#"
    |"part 1\\part 2"
  "#,

  string_escape_newline: r#"
    |"line 1\nline 2"
  "#,

  string_escape_return: r#"
    |"line 1\rline 2"
  "#,

  string_escape_tab: r#"
    |"hello\tworld"
  "#,

  string_emoji: r#"
    |"frog 🐸"
  "#,

  string_emoji_2: r#"
    |"🌝"
  "#,

  string_emoji_3: r#"
    |"🏳️‍🌈"
  "#,

  string_unicode: r#"
    |"this is u̲n̲d̲e̲r̲l̲i̲n̲e̲d̲ with unicode chars"
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

  identifier_not_ascii: r#"
    |こんにちは
  "#,

  parenthesized: r#"
    |(1)
  "#,

  tuple_empty: r#"
    |()
  "#,

  tuple_two_elements: r#"
    |(1, "wow")
  "#,

  tuple_multiple_elements: r#"
    |(1, "wow", { x => x }, (lol, ()))
  "#,

  tuple_empty_across_lines: r#"
    |(
    |
    |)
  "#,

  tuple_across_lines: r#"
    |(1,
    | 2
    |)
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

  def_generic_type_constraint: r#"
    |def hello A -> A where A :: Any {
    |  x => x
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

  binary_op_plus: r#"
    |5 + 5
  "#,

  binary_op_plusplus: r#"
    |"a" ++ "b"
  "#,

  binary_op_less_than: r#"
    |"a" < "b"
  "#,

  binary_op_greater_than: r#"
    |"a" > "b"
  "#,

  private_defs: r#"
    |def isPublic() {}
    |
    |private
    |
    |def isPrivate() {}
  "#,

  let_pattern_identifier: r#"
    |let x = 47
  "#,

  let_followed_by_expr: r#"
    |let x = "wow"
    |
    |x
  "#,

  let_pattern_underscore: r#"
    |let _ = 47
  "#,

  let_pattern_tuple: r#"
    |let (a, b) = (47, "cool")
  "#,

  let_pattern_nested_tuples: r#"
    |let (a, _, (_, b)) = (47, something, tuple)
  "#,

  let_pattern_struct_constructor: r#"
    |let Person (name, age) = Person ("Reid", 26)
  "#,

  match_pattern_same_line: r#"
    |match thing | 1 => "one" | 2 => "two" | _ => "idk"
  "#,

  match_pattern_across_lines: r#"
    |match thing
    |  | (a, b) => "one"
    |  | (1, _) => "two"
    |  | _ => "idk"
  "#,

  type_assertion_basic: r#"
    |1 :: Int
  "#,

  type_assertion_complex: r#"
    |f :: { Int -> { () -> (Bool, Int) } }
  "#,

  type_enum_same_line: r#"
    |enum Bool | True | False
  "#,

  type_enum_across_lines: r#"
    |enum Color
    |  | Red
    |  | Green
    |  | Blue
  "#,

  type_enum_constructor_args: r#"
    |enum Thing
    |  | Wow Int
    |  | Cool(String)
    |  | NoArg
    |  | TupleArg (Int, String)
  "#,

  type_enum_generic_constraints: r#"
    |enum Optional<A> where A :: Any
    |  | Some(A)
    |  | None
  "#,

  type_struct_same_line: r#"
    |struct Person (name :: String, age :: Int)
  "#,

  type_struct_across_lines: r#"
    |struct Person (
    |  name :: String,
    |  age :: Int
    |)
  "#,

  type_struct_generic_constraints: r#"
    |struct Thing<A, B> where A :: Any, B :: Comparable (
    |  first :: A,
    |  second :: B,
    |  third :: Int
    |)
  "#,

  type_alias_same_line: r#"
    |alias Alpha Beta
  "#,

  type_alias_across_lines: r#"
    |alias Alpha
    |  Beta
  "#,

  type_alias_generic_constraints: r#"
    |alias CoolFunc<A> where A :: Any
    |  { (A, Int) -> Cool }
  "#,

  type_trait_same_line_one_field: r#"
    |trait HasName . name :: String
  "#,

  type_trait_same_line_two_fields: r#"
    |trait HasNameAndAge . name :: String . age :: Int
  "#,

  type_trait_across_lines_two_fields: r#"
    |trait HasNameAndAge
    |  . name :: String
    |  . age :: Int
  "#,

  type_trait_two_methods: r#"
    |trait Wowie
    |  . getWow () -> Wow
    |  . setWow Wow -> ()
  "#,

  type_trait_mix_fields_and_methods: r#"
    |trait WowieWithName
    |  . getWow () -> Wow
    |  . setWow Wow -> ()
    |  . name :: String
  "#,
}
