use crate::ast::{Node, NodeType};
use crate::tokens::Token;
use std::collections::HashMap;

#[derive(Debug)]
pub struct PackageCompilationErrorSummary {
  pub package_errors: Vec<String>,
  pub module_errors: HashMap<String, Vec<ModuleCompilationErrorDetail>>,
}

#[derive(Debug)]
pub struct ModuleCompilationErrorDetail {
  pub location: Option<(usize, usize)>,
  pub module_path: String,
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
  AnalysisError(AnalysisError),
}

#[derive(Debug)]
pub enum FileError {
  FailedToReadFile(String),
}

#[derive(Debug)]
pub enum TokenizeError {
  InvalidDecimalDigit(usize, usize),
  InvalidBinaryDigit(usize, usize),
  InvalidHexDigit(usize, usize),
  InvalidOctalDigit(usize, usize),
  UnclosedString(usize, usize),
  UnclosedInterpolation(usize, usize),
}

#[derive(Debug, Clone)]
pub enum ParseError {
  UnexpectedToken(Token),
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
  MissingConstraintsAfterWhere(usize),
}

#[derive(Debug, Clone)]
pub enum AnalysisError {
  TypeMismatch(Node, NodeType, NodeType),
  TypeMismatchArrayElement(Node, NodeType, NodeType),
  UndefinedVariable(Node),
  UndefinedQualifier(Node),
}
