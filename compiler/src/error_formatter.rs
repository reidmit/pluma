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
          let module_result = self.compiler.modules.get(module_path).unwrap();

          if let Err(module_err) = module_result {
            let errors = vec![
              self.get_module_error_details(&module_path, module_err),
            ];

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

    ModuleCompilationErrorDetail {
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
        &TokenizeError::InvalidBinaryDigitError(start, end) =>
          (
            (start, end),
            format!("Invalid binary digit: {}", self.get_source_string(module_path, start, end)),
          ),

        _ => unimplemented!()
      },
      _ => unreachable!()
    };

    (Some(location), message)
  }

  fn get_parse_error_details(&self, _err: &ModuleCompilationError) -> (Option<(usize, usize)>, String) {
    let location = None;
    let message = "parse_error".to_owned();

    (location, message)
  }

  fn get_source_string(&self, _module_path: &String, _start: usize, _end: usize) -> String {
    // TODO
    return "test".to_owned()
  }
}