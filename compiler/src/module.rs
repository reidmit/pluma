use crate::fs;
use crate::ast::Node;
use crate::parser::Parser;
use crate::tokenizer::{Tokenizer, TokenList, CommentMap};
use crate::errors::ModuleCompilationError;

#[derive(Debug)]
pub struct Module {
  abs_path: String,
  rel_path: String,
  bytes: Option<Vec<u8>>,
  tokens: Option<TokenList>,
  comments: Option<CommentMap>,
  ast: Option<Node>,
}

impl Module {
  pub fn new(abs_path: String, rel_path: String) -> Module {
    Module {
      abs_path,
      rel_path,
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

    Ok(())
  }

  pub fn get_referenced_paths(&self) -> Vec<String> {
    match &self.ast {
      Some(ast) => {
        let mut paths = Vec::new();

        if let Node::Module { imports, .. } = ast {
          for import in imports {
            if let Node::Import { path, .. } = import {
              paths.push(path.clone());
            }
          }
        }

        paths
      },
      None => panic!("called before module.parse()")
    }
  }

  fn read(&mut self) -> Result<(), ModuleCompilationError> {
    match fs::read_file_contents(&self.abs_path) {
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
      None => panic!("called before module.read()")
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
      (None, _) => panic!("called before module.read()"),
      (_, None) => panic!("called before module.tokenize()")
    }
  }
}