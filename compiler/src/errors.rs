use std::fmt;
use crate::ast::Node;

#[derive(Debug)]
pub enum PackageCompilationError {
  ConfigInvalid(ConfigurationError),
  ModuleFailedToCompile(ModuleCompilationError),
}

#[derive(Debug)]
pub enum ConfigurationError {
  EntryPathDoesNotExist(String),
}

#[derive(Debug)]
pub enum ModuleCompilationError {
  FileError(FileError),
  TokenizeError(TokenizeError),
  ParseError(ParseError),
}

#[derive(Debug)]
pub enum FileError {
  FailedToReadFile(String),
}

#[derive(Debug)]
pub enum TokenizeError {
  InvalidBinaryDigitError(usize, usize),
  InvalidHexDigitError(usize, usize),
  InvalidOctalDigitError(usize, usize),
  UnclosedStringError(usize, usize),
  UnclosedInterpolationError(usize, usize),
}

#[derive(Debug, Clone)]
pub enum ParseError {
  UnexpectedToken(usize),
  UnexpectedEOF,
  UnclosedParentheses(usize),
  UnclosedBlock(usize),
  UnclosedArray(usize),
  UnclosedDict(usize),
  UnexpectedArrayElementInDict(Node),
  UnexpectedDictEntryInArray(Node),
  UnexpectedTokenAfterDot(usize),
  UnexpectedTokenInImport(usize),
  MissingArrowInMatchCase(usize),
  MissingArrowAfterBlockParams(usize),
  MissingAliasAfterAsInImport(usize),
  MissingCasesInMatchExpression(usize),
}

#[derive(Debug)]
pub struct UsageError {
  message: String,
}

impl fmt::Display for UsageError {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "{}", self.message)
  }
}

impl UsageError {
  pub fn unknown_command(command_name: String) -> UsageError {
    UsageError {
      message: format!("Unknown command: {}", command_name),
    }
  }
}
