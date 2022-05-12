use pencil::*;

fn main() {
  let entry_path = match std::env::args().nth(1) {
    Some(path) => path,
    None => panic!("no arg given"),
  };

  let source = std::fs::read(entry_path).unwrap();
  let tokenizer = Tokenizer::from_source(&source);
  let mut parser = Parser::new(&source, tokenizer);

  let (module, comments, errors) = parser.parse_module();

  if comments.len() > 0 {
    println!("comments: {:#?}", comments);
  }

  println!("{:#?}", module);

  if errors.len() > 0 {
    println!("errors: {:#?}", errors);
  }
}
