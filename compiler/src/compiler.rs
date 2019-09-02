use crate::config::CompilerConfig;
use crate::fs;
use crate::parser::Parser;
use crate::tokenizer::{Tokenizer, TokenizeResult};

pub struct Compiler {
  entry_path: String,
}

impl Compiler {
  pub fn new(config: CompilerConfig) -> Compiler {
    Compiler {
      entry_path: config.entry_path.clone(),
    }
  }

  pub fn compile_module(&self, source: Vec<u8>) {
    match Tokenizer::from_source(&source).collect_tokens() {
      TokenizeResult::TokenList(tokens) => {
        let mut parser = Parser::from_tokens(&tokens);
        let ast = parser.parse_module();
        println!("{:#?}", ast);
      }

      _ => {
        panic!("Tokenizer error!"); // TODO
      }
    }
  }

  pub fn add_module(&self, abs_file_path: &String) -> Result<bool, String> {
    match fs::read_file_contents(abs_file_path) {
      Ok(contents) => {
        self.compile_module(contents);
        Ok(true)
      }
      Err(_) => Ok(false),
    }
  }

  pub fn run(&self) -> Result<(), String> {
    let entry = &self.entry_path;
    let _ = self.add_module(entry);

    Ok(())
  }
}
