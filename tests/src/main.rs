use pluma_compiler::*;
use std::collections::HashMap;
use std::fs::{self, DirEntry};
use std::io;
use std::path::Path;

#[derive(Debug)]
enum ModuleAnalysisTestResult {
  UnexpectedDiagnostic(Diagnostic),
  ExpectedWarningButGotError(Diagnostic),
  ExpectedErrorButGotWarning(Diagnostic),
  MissingError { line: usize, comment: String },
  MissingWarning { line: usize, comment: String },
}

fn main() -> io::Result<()> {
  // must be run from workspace root:
  let cases_dir = Path::new("tests/cases").canonicalize()?;
  let mut all_results = HashMap::new();

  visit_dirs(&cases_dir, &mut all_results, &|entry, all_results| {
    let current_dir = std::env::current_dir().unwrap();
    let entry_path = &entry.path();
    let short_path = entry_path.strip_prefix(current_dir).unwrap();

    let module_results = check_module(entry_path);

    all_results.insert(path_to_string(&short_path), module_results);
  })?;

  let mut failures = 0;

  for (path, module_results) in all_results {
    if module_results.is_empty() {
      println!("\x1b[32m✔ {}\x1b[0m", path);
    } else {
      eprintln!("\x1b[31m𝙭 {}\x1b[0m", path);
      eprintln!("{:#?}", module_results);
      failures += 1;
    }
  }

  if failures > 0 {
    eprintln!("\n\x1b[31m𝙭 FAILED: {} errors\x1b[0m", failures);
    std::process::exit(47);
  }

  println!("\n\x1b[32m✔ All passed!\x1b[0m");

  Ok(())
}

fn path_to_string(path: &Path) -> String {
  path.to_string_lossy().to_owned().to_string()
}

fn check_module(path: &Path) -> Vec<ModuleAnalysisTestResult> {
  let mut compiler = Compiler::from_options(CompilerOptions {
    entry_path: path_to_string(path),
    mode: CompilerMode::Debug,
    output_path: None,
  })
  .expect("compiler from options");

  let mut module_results = Vec::new();

  // First, parse/analyze and get the results/comments
  let check_result = compiler.check();
  let parsed_module = &compiler.modules[&compiler.entry_module_name];
  let parsed_comments = &mut parsed_module.comments.clone();

  // Then, for each diagnostic that came back, check if we had an expect comment for it:
  if let Err(diagnostics) = check_result {
    for diagnostic in diagnostics {
      let line_number =
        parsed_module.get_line_for_position(diagnostic.pos.expect("pos for diagnostic"));

      if let Some(comment_for_line) = parsed_comments.get(&line_number) {
        if comment_for_line.starts_with(" expect-error") {
          if diagnostic.is_error() {
            // OK, we expected an error here. Remove it from the map
            // so we don't count it again later.
            parsed_comments.remove(&line_number);
          } else {
            // We expected an error, but got a warning:
            module_results.push(ModuleAnalysisTestResult::ExpectedErrorButGotWarning(
              diagnostic,
            ))
          }
        } else if comment_for_line.starts_with(" expect-warning") {
          if diagnostic.is_error() {
            // We expected a warning, but got an error:
            module_results.push(ModuleAnalysisTestResult::ExpectedWarningButGotError(
              diagnostic,
            ))
          } else {
            // OK, we expected a warning here. Remove it from the map
            // so we don't count it again later.
            parsed_comments.remove(&line_number);
          }
        } else {
          module_results.push(ModuleAnalysisTestResult::UnexpectedDiagnostic(diagnostic));
        }
      } else {
        module_results.push(ModuleAnalysisTestResult::UnexpectedDiagnostic(diagnostic));
      }
    }
  }

  // Then, check if we have any comments that did not result in diagnostics:
  for (line_number, comment) in parsed_comments {
    if comment.starts_with(" expect-error") {
      module_results.push(ModuleAnalysisTestResult::MissingError {
        line: *line_number,
        comment: comment.to_string(),
      })
    } else if comment.starts_with(" expect-warning") {
      module_results.push(ModuleAnalysisTestResult::MissingWarning {
        line: *line_number,
        comment: comment.to_string(),
      })
    }
  }

  module_results
}

fn visit_dirs(
  dir: &Path,
  results: &mut HashMap<String, Vec<ModuleAnalysisTestResult>>,
  cb: &dyn Fn(&DirEntry, &mut HashMap<String, Vec<ModuleAnalysisTestResult>>),
) -> io::Result<()> {
  if dir.is_dir() {
    for entry in fs::read_dir(dir)? {
      let entry = entry?;
      let path = entry.path();

      if path.is_dir() {
        visit_dirs(&path, results, cb)?;
      } else {
        cb(&entry, results);
      }
    }
  }

  Ok(())
}
