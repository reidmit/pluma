mod helpers;

const SOURCE: &str = r#"
main = fun a b c ->
  a + b * c
"#;

pub fn main() {
  helpers::parse_and_print(&SOURCE);
}
