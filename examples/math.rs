mod helpers;

const SOURCE: &str = r#"
main = \
  let sum = 1 + 2;
  let product = sum * 100;
  product ** 120 - 10
"#;

pub fn main() {
  helpers::parse_and_print(&SOURCE);
}
