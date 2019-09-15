use std::collections::HashMap;
use crate::compiler::Compiler;
use crate::errors::*;

pub struct ErrorFormatter<'a> {
  compiler: &'a Compiler,
  error: PackageCompilationError
}

impl<'a> ErrorFormatter<'a> {
  pub fn new(compiler: &'a Compiler, error: PackageCompilationError) -> Self {
    ErrorFormatter {
      compiler,
      error,
    }
  }

  pub fn get_error_summary(&self) -> PackageCompilationErrorSummary {
    let mut module_errors = HashMap::new();
    let package_errors = Vec::new();

    match &self.error {
      PackageCompilationError::ModulesFailedToCompile(modules_with_errors) => {
        for module_path in modules_with_errors {
          let module = self.compiler.modules.get(module_path).unwrap();

          if module.has_errors() {
            let mut errors = Vec::new();

            for module_error in &module.errors {
              errors.push(
                self.get_module_error_details(&module_path, &module_error)
              )
            }

            module_errors.insert(module_path.clone(), errors);
          }
        }
      },

      _ => unimplemented!()
    }

    PackageCompilationErrorSummary {
      module_errors,
      package_errors,
    }
  }

  fn get_module_error_details(&self, module_path: &String, err: &ModuleCompilationError) -> ModuleCompilationErrorDetail {
    let (location, message) = match err {
      ModuleCompilationError::FileError(..) => self.get_file_error_details(err),
      ModuleCompilationError::TokenizeError(..) => self.get_tokenize_error_details(module_path, err),
      ModuleCompilationError::ParseError(..) => self.get_parse_error_details(err),
    };

    let module = self.compiler.modules.get(module_path).unwrap();
    let abs_path = module.abs_path.to_string();

    ModuleCompilationErrorDetail {
      module_path: abs_path,
      location,
      message,
    }
  }

  fn get_file_error_details(&self, err: &ModuleCompilationError) -> (Option<(usize, usize)>, String) {
    let location = None;

    let message = match err {
      ModuleCompilationError::FileError(file_err) => match file_err {
        FileError::FailedToReadFile(file_name) =>
          format!("Failed to read file: {}", file_name)
      },
      _ => unreachable!()
    };

    (location, message)
  }

  fn get_tokenize_error_details(&self, module_path: &String, err: &ModuleCompilationError) -> (Option<(usize, usize)>, String) {
    let (location, message) = match err {
      ModuleCompilationError::TokenizeError(tok_err) => match tok_err {
         &TokenizeError::InvalidDecimalDigit(start, end) =>
          (
            (start, end),
            format!(
              "Invalid digit `{}` in number. Valid digits are `0` to `9`.",
              self.read_source(module_path, start, end),
            ),
          ),

        &TokenizeError::InvalidBinaryDigit(start, end) =>
          (
            (start, end),
            format!(
              "Invalid digit `{}` in binary number. Valid binary digits are `0` and `1`.",
              self.read_source(module_path, start, end),
            ),
          ),

        &TokenizeError::InvalidHexDigit(start, end) =>
          (
            (start, end),
            format!(
              "Invalid digit `{}` in hexadecimal number. Valid hexadecimal digits are `0` to `9` and `a` to `f`.",
              self.read_source(module_path, start, end),
            ),
          ),

        &TokenizeError::InvalidOctalDigit(start, end) =>
          (
            (start, end),
            format!(
              "Invalid digit `{}` in octal number. Valid octal digits are `0` to `7`.",
              self.read_source(module_path, start, end),
            ),
          ),

        &TokenizeError::UnclosedString(start, _) =>
          (
            (start, start + 1),
            format!(
              "Unclosed string",
            ),
          ),

        &TokenizeError::UnclosedInterpolation(start, _) =>
          (
            (start, start + 1),
            format!(
              "Unclosed string interpolation",
            ),
          ),
      },
      _ => unreachable!()
    };

    (Some(location), message)
  }

  fn get_parse_error_details(&self, err: &ModuleCompilationError) -> (Option<(usize, usize)>, String) {
    let (location, message) = match err {
      ModuleCompilationError::ParseError(parse_err) => match parse_err {
        er => (None, format!("{:#?}", er))
      },
      _ => unreachable!()
    };

    (location, message)
  }

  fn read_source(&self, module_path: &String, start: usize, end: usize) -> String {
    let module = self.compiler.modules.get(module_path).unwrap();

    match &module.bytes {
      Some(bytes) => String::from_utf8(bytes[start..end].to_vec()).expect("not utf8"),
      None => "".to_owned()
    }
  }
}