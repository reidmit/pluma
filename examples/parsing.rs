mod helpers;

const SOURCE: &str = r#"
tokenize bytes = \
  let tokens = []
  let count = 0

  bytes | each \ byte ->
    byte
      ? 'a' -> do \
        count <- count + 1
        tokens | push (token/a "wow")

      ? 'b' -> print "it's b"

      ? 'c' -> print "it's c"

      ? _   -> done

  tokens

main args = \
  let bytes = core/fs/read-file args[0]
  let tokens = tokenize bytes
  let ast = parse tokens
  print ast
"#;

pub fn main() {
  helpers::parse_and_print(&SOURCE);
}
