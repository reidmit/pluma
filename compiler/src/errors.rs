use std::collections::HashMap;
use crate::ast::Node;

#[derive(Debug)]
pub struct PackageCompilationErrorSummary {
  pub package_errors: Vec<String>,
  pub module_errors: HashMap<String, Vec<ModuleCompilationErrorDetail>>
}

#[derive(Debug)]
pub struct ModuleCompilationErrorDetail {
  pub location: Option<(usize, usize)>,
  pub message: String,
}

#[derive(Debug)]
pub enum PackageCompilationError {
  ModulesFailedToCompile(Vec<String>),
  CyclicalDependency(Vec<String>),
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