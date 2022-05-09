mod helpers;

const SOURCE: &str = r#"
fizzbuzz = \ n ->
  (n % 3, n % 5) {
    ? (0, 0) -> "FizzBuzz"
    ? (0, _) -> "Fizz"
    ? (_, 0) -> env.get "LIGHTYEAR" {
      ? Some(val) -> val
      ? None      -> "Buzz"
    }
    ? _ -> n | stringify
  }

main = \
  1 .. 100 | each \ i ->
    fizzbuzz i | print
"#;

pub fn main() {
  helpers::parse_and_print(&SOURCE);
}
