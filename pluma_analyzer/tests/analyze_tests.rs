#[macro_use]
mod macros;

test_analyze! {
  undefined_identifier_simple (false): r#"
    |wat
  "#,

  defined_identifier_simple (true): r#"
    |let fine = 47
    |
    |fine
  "#,

  intrinsic_def_binary_op_valid (true): r#"
    |intrinsic_type Int
    |intrinsic_def Int + Int -> Int
    |
    |let result = 3 + 4
  "#,

  intrinsic_def_binary_op_invalid (false): r#"
    |intrinsic_type Int
    |intrinsic_def Int + Int -> Int
    |
    |let result = 3 + "yikes"
  "#,

  enum_type_no_args (true): r#"
    |enum Color | Red | Green | Blue
    |
    |let r = Red
    |let g = Green
  "#,

  enum_type_constructor (true): r#"
    |intrinsic_type Int
    |intrinsic_type String
    |
    |enum Either
    | | Left(Int)
    | | Right(String)
    |
    |let a = Left(47)
    |let b = Right("hi")
  "#,

  enum_type_constructor_invalid_args (false): r#"
    |intrinsic_type Int
    |intrinsic_type String
    |
    |enum Either
    | | Left(Int)
    | | Right(String)
    |
    |let a = Left("hi")
    |let b = Right(47)
  "#,

  let_unlabeled_tuple_pattern (true): r#"
    |let tup = (47, "wow", 1.23)
    |let (a, b, c) = tup
    |
    |let tup2 = (47, ("wow", 1.23))
    |let (d, (e, f)) = tup2
  "#,

  let_labeled_tuple_pattern (true): r#"
    |intrinsic_type Int
    |intrinsic_type String
    |
    |let tup = (name: "Reid", age: 26)
    |let (name: name2, age: age2) = tup
    |
    |tup :: (name :: String, age :: Int)
    |name2 :: String
    |age2 :: Int
  "#,

  let_labeled_tuple_pattern_out_of_order (true): r#"
    |intrinsic_type Int
    |intrinsic_type String
    |
    |let tup = (name: "Reid", age: 26)
    |let (age: age2, name: name2) = tup
    |
    |tup :: (name :: String, age :: Int)
    |name2 :: String
    |age2 :: Int
  "#,

  let_labeled_tuple_unknown_field (false): r#"
    |intrinsic_type Int
    |intrinsic_type String
    |
    |let tup = (name: "Reid", age: 26)
    |let (age: age2, wat: name2) = tup
  "#,

  def_function (true): r#"
    |intrinsic_type Int
    |
    |def returnsNothing () {
    |  arg => ()
    |}
    |
    |def intToInt Int -> Int {
    |  x => x
    |}
    |
    |let void = returnsNothing()
    |let n = intToInt 100
  "#,

  undefined_type_in_signature (false): r#"
    |def takesSomething Wat {
    |  arg => ()
    |}
  "#,

  undefined_return_type_in_signature (false): r#"
    |def takesSomething () -> Wat {
    |  arg => ()
    |}
  "#,

  def_method (true): r#"
    |enum Color | Red | Green | Blue
    |
    |def Color . funk Color -> Color {
    |  self, b => b
    |}
    |
    |let c = Blue
    |c.funk(Red)
  "#,

  type_assertions_valid (true): r#"
    |intrinsic_type Int
    |intrinsic_type String
    |
    |let n = 47 :: Int
    |let s = "lol" :: String
  "#,

  type_assertions_invalid (false): r#"
    |intrinsic_type Int
    |intrinsic_type String
    |
    |let n = 47 :: String
    |let s = "lol" :: Int
  "#,
}
