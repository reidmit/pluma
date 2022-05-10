mod helpers;

const SOURCE: &str = r#"
haha
  # lol ???
  # line 2 haha
  = 0xdeadbeef
"#;

pub fn main() {
  helpers::parse_and_print(&SOURCE);
}
