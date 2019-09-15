use crate::fs;
use crate::ast::Node;
use crate::parser::Parser;
use crate::tokens::{Token};
use crate::tokenizer::{Tokenizer, TokenizeResult, TokenList, CommentMap};
use crate::errors::{PackageCompilationError, ModuleCompilationError};

pub struct Module {
  path: String,
  bytes: Option<Vec<u8>>,
  tokens: Option<TokenList>,
  comments: Option<CommentMap>,
  ast: Option<Node>,
}

impl Module {
  pub fn new(path: String) -> Module {
    Module {
      path,
      bytes: None,
      tokens: None,
      comments: None,
      ast: None,
    }
  }

  pub fn compile(&mut self) -> Result<(), ModuleCompilationError> {
    self.read()?;
    self.tokenize()?;
    self.parse()?;

    println!("tokens: {:#?}", self.tokens);
    println!("ast: {:#?}", self.ast);

    Ok(())
  }

  fn read(&mut self) -> Result<(), ModuleCompilationError> {
    match fs::read_file_contents(&self.path) {
      Ok(bytes) => Ok(self.bytes = Some(bytes)),
      Err(err) => Err(ModuleCompilationError::FileError(err))
    }
  }

  fn tokenize(&mut self) -> Result<(), ModuleCompilationError> {
    match &self.bytes {
      Some(bytes) => {
        match Tokenizer::from_source(bytes).collect_tokens() {
          Ok((tokens, comments)) => {
            self.tokens = Some(tokens);
            self.comments = Some(comments);
            Ok(())
          },
          Err(err) => Err(ModuleCompilationError::TokenizeError(err))
        }
      },
      None => panic!("module.tokenize() called before module.read()")
    }
  }

  fn parse(&mut self) -> Result<(), ModuleCompilationError> {
    match (&self.bytes, &self.tokens) {
      (Some(source), Some(tokens)) => {
        match Parser::new(source, tokens).parse_module() {
          Ok(ast) => Ok(self.ast = Some(ast)),
          Err(err) => Err(ModuleCompilationError::ParseError(err))
        }
      },
      (None, _) => panic!("module.parse() called before module.read()"),
      (_, None) => panic!("module.parse() called before module.tokenize()")
    }
  }

  pub fn get_comment_for_line(&self, line: usize) -> Option<&Token> {
    match &self.comments {
      Some(comment_map) => comment_map.get(&line),
      None => None
    }
  }

  pub fn get_referenced_paths(&self) -> Vec<String> {
    Vec::new()
  }
}