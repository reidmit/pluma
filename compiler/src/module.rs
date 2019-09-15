use crate::fs;
use crate::ast::Node;
use crate::parser::Parser;
use crate::tokenizer::{Tokenizer, TokenList, CommentMap};
use crate::errors::ModuleCompilationError;

#[derive(Debug)]
pub struct Module {
  pub abs_path: String,
  pub rel_path: String,
  pub bytes: Option<Vec<u8>>,
  tokens: Option<TokenList>,
  comments: Option<CommentMap>,
  ast: Option<Node>,
  pub errors: Vec<ModuleCompilationError>,
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
      errors: Vec::new(),
    }
  }

  pub fn compile(&mut self) {
    if let Err(err) = self.read() {
      self.errors.push(err);
      return;
    }

    if let Err(err) = self.tokenize() {
      self.errors.push(err);
      return;
    }

    if let Err(err) = self.parse() {
      self.errors.push(err);
      return;
    }
  }

  pub fn has_errors(&self) -> bool {
    self.errors.len() > 0
  }

  pub fn get_referenced_paths(&self) -> Option<Vec<String>> {
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

        Some(paths)
      },
      None => None
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
      _ => unreachable!()
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
      _ => unreachable!()
    }
  }
}