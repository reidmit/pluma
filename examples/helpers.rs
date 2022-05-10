use pencil::*;

pub fn parse_and_print(source: &str) {
  let bytes = source.as_bytes().into();
  let tokenizer = Tokenizer::from_source(&bytes);

  // for token in tokenizer {
  //   println!("{:#?}", token)
  // }

  let mut parser = Parser::new(&bytes, tokenizer);

  let (module, comments, errors) = parser.parse_module();

  if comments.len() > 0 {
    println!("comments: {:#?}", comments);
  }

  println!("{:#?}", module);

  if errors.len() > 0 {
    println!("errors: {:#?}", errors);
  }
}

#[allow(dead_code)]
pub fn main() {
  println!(":-----)")
}
