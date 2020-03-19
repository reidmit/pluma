// use crate::compiler::Compiler;
// use crate::errors::*;
// use std::collections::HashMap;

// pub struct ErrorFormatter<'a> {
//   compiler: &'a Compiler,
//   error: PackageCompilationError,
// }

// impl<'a> ErrorFormatter<'a> {
//   pub fn new(compiler: &'a Compiler, error: PackageCompilationError) -> Self {
//     ErrorFormatter { compiler, error }
//   }

//   pub fn get_error_summary(&self) -> PackageCompilationErrorSummary {
//     let mut module_errors = HashMap::new();
//     let mut package_errors = Vec::new();

//     match &self.error {
//       PackageCompilationError::ModulesFailedToCompile(modules_with_errors) => {
//         for module_path in modules_with_errors {
//           let module = self.compiler.modules.get(module_path).unwrap();

//           if module.has_errors() {
//             let mut errors = Vec::new();

//             for module_error in &module.errors {
//               errors.push(self.get_module_error_details(&module_path, &module_error))
//             }

//             module_errors.insert(module_path.clone(), errors);
//           }
//         }
//       }

//       PackageCompilationError::CyclicalDependency(cycle) => package_errors.push(format!(
//         "Cyclical dependencies between modules:\n\n{}",
//         cycle.join(" --> ")
//       )),
//     }

//     PackageCompilationErrorSummary {
//       module_errors,
//       package_errors,
//     }
//   }

//   fn get_module_error_details(
//     &self,
//     module_name: &String,
//     err: &ModuleCompilationError,
//   ) -> ModuleCompilationErrorDetail {
//     let (location, message) = (None, format!("{:#?}", err));
//     let module = self.compiler.modules.get(module_name).unwrap();
//     let module_path = module.module_path.to_string();

//     ModuleCompilationErrorDetail {
//       module_path,
//       location,
//       message,
//     }
//   }

//   // fn read_source(&self, module_path: &String, start: usize, end: usize) -> String {
//   //   let module = self.compiler.modules.get(module_path).unwrap();

//   //   match &module.bytes {
//   //     Some(bytes) => String::from_utf8(bytes[start..end].to_vec()).expect("not utf8"),
//   //     None => "".to_owned(),
//   //   }
//   // }
// }
