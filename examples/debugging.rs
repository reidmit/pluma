mod helpers;

const SOURCE: &str = r#"
re = `
  "hello"
  digit{2}
  "world"
`

main = ((1, x:0), 2, c: 3)
"#;

pub fn main() {
  helpers::parse_and_print(&SOURCE);
}
