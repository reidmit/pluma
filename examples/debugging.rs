mod helpers;

const SOURCE: &str = r#"
main = fun a b c ->
  if a && b && c | print
    is true then "eo"
    is false then "nooo"
"#;

pub fn main() {
  helpers::parse_and_print(&SOURCE);
}
