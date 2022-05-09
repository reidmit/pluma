mod helpers;

const SOURCE: &str = r#"
haha = 0xdeadbeef
"#;

pub fn main() {
  helpers::parse_and_print(&SOURCE);
}
