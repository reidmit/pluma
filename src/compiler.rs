use config::CompilerConfig;
use fs;
use parser::Parser;
use std::collections::HashMap;
use tokenizer::Tokenizer;

pub struct Compiler<'a> {
  entry_path: String,
  preserve_comments: bool,
  modules: HashMap<String, Parser<'a>>,
}

impl<'a> Compiler<'a> {
  pub fn new(config: CompilerConfig) -> Compiler<'a> {
    Compiler {
      entry_path: config.entry_path.clone(),
      preserve_comments: config.preserve_comments,
      modules: HashMap::new(),
    }
  }

  pub fn compile_module(&self, file_contents: Vec<u8>) {
    let mut parser = Parser::from_source(&file_contents, self.preserve_comments);
    let ast = parser.parse_module();

    println!("{:#?}", ast);
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

    // println!("{}", result.unwrap());

    Ok(())
  }
}
