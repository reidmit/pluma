mod helpers;

const SOURCE: &str = r#"
# = 3735928559
# = 2147483647
haha = 0xdeadbeef
"#;

pub fn main() {
  helpers::parse_and_print(&SOURCE);
}
