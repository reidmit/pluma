// use ast::*;
// use diagnostics::*;
// use inkwell::builder::Builder;
// use inkwell::context::Context;
// use inkwell::module::{Linkage, Module};
// use inkwell::passes::PassManager;
// use inkwell::targets::TargetTriple;
// use inkwell::values::*;
// use inkwell::{AddressSpace, OptimizationLevel};
// use std::convert::TryInto;
// use std::io::prelude::*;
// use std::process::{Command, Stdio};
// use visitor::*;

// pub struct Emitter<'ctx> {
//   llvm_context: &'ctx Context,
//   llvm_builder: Builder<'ctx>,
//   llvm_module: Module<'ctx>,
//   main_function: FunctionValue<'ctx>,
// }

// impl<'ctx> Emitter<'ctx> {
//   pub fn create_context() -> Context {
//     Context::create()
//   }

//   pub fn new(llvm_context: &'ctx Context) -> Emitter<'ctx> {
//     let llvm_builder = llvm_context.create_builder();
//     let llvm_module = llvm_context.create_module("root_module");

//     llvm_module.set_triple(&TargetTriple::create("x86_64-apple-macosx10.15.0"));

//     let return_type = llvm_context.i32_type().fn_type(&Vec::new(), false);
//     let main_function = llvm_module.add_function("main", return_type, None);

//     return Emitter {
//       llvm_context,
//       llvm_builder,
//       llvm_module,
//       main_function,
//     };
//   }

//   pub fn verify(&self) -> Result<(), Diagnostic> {
//     self
//       .llvm_module
//       .verify()
//       .map_err(|err_msg| Diagnostic::error(err_msg))
//   }

//   pub fn write_to_string(&self) -> String {
//     self.llvm_module.print_to_string().to_string()
//   }

//   pub fn write_to_path(&self, path: &std::path::Path) -> Result<(), Diagnostic> {
//     let mut process = Command::new("clang")
//       .args(&["-x", "ir", "-", "-o", path.to_str().unwrap()])
//       .stdin(Stdio::piped())
//       .stdout(Stdio::piped())
//       .spawn()
//       .map_err(|_| Diagnostic::error("Failed to spawn process."))?;

//     let child_stdin = process.stdin.as_mut().unwrap();

//     child_stdin
//       .write_all(&self.write_to_string().as_bytes()[..])
//       .map_err(|_| Diagnostic::error("Failed to pipe input to process."))?;

//     let output = process
//       .wait_with_output()
//       .map_err(|_| Diagnostic::error("Problem waiting for process to exit."))?;

//     if !output.status.success() {
//       return Err(Diagnostic::error("Process did not terminate successfully."));
//     }

//     Ok(())
//   }

//   pub fn optimize(&mut self) {
//     let llvm_pass_manager = PassManager::create(&self.llvm_module);
//     // TODO: add passes!
//     llvm_pass_manager.initialize();

//     llvm_pass_manager.run_on(&mut self.main_function);
//   }

//   pub fn execute(&self) -> i32 {
//     let execution_engine = self
//       .llvm_module
//       .create_jit_execution_engine(OptimizationLevel::None)
//       .unwrap();

//     let exit_code = unsafe { execution_engine.run_function_as_main(self.main_function, &[]) };

//     exit_code
//   }

//   fn compile_call(&self, call: &CallNode) -> BasicValueEnum {
//     // let callee_name = match &call.callee.kind {
//     //   ExprKind::Identifier { ident } => ident.name.clone(),
//     //   _ => todo!(),
//     // };
//     let callee_name = "TODO_IDK_AAHHH".to_owned();

//     let func = self
//       .llvm_module
//       .get_function(&callee_name[..])
//       .expect("should have function defined");

//     let mut compiled_args = Vec::with_capacity(1);

//     for arg in &call.args {
//       compiled_args.push(self.compile_expr(&arg));
//     }

//     let argsv: Vec<BasicValueEnum> = compiled_args
//       .iter()
//       .by_ref()
//       .map(|&val| val.into())
//       .collect();

//     let call_site_value = self.llvm_builder.build_call(func, argsv.as_slice(), "tmp");

//     call_site_value
//       .try_as_basic_value()
//       .left()
//       .expect("call did not return a basic value")
//   }

//   fn compile_expr(&self, expr: &ExprNode) -> BasicValueEnum {
//     match &expr.kind {
//       ExprKind::Literal { literal } => self.compile_literal(literal),

//       ExprKind::Call { call } => self.compile_call(call),

//       _other => todo!("compile expr kind"),
//     }
//   }

//   fn compile_literal(&self, lit: &LiteralNode) -> BasicValueEnum {
//     match &lit.kind {
//       LiteralKind::IntDecimal(value) => self
//         .llvm_context
//         .i32_type()
//         .const_int((*value).try_into().unwrap(), true)
//         .into(),

//       LiteralKind::FloatDecimal(value) => self.llvm_context.f64_type().const_float(*value).into(),

//       LiteralKind::Str(value) => {
//         // hmm
//         let global_value = self
//           .llvm_builder
//           .build_global_string_ptr(value.as_str(), "str");

//         global_value.as_pointer_value().into()
//       }

//       _other => todo!("compile literal kind"),
//     }
//   }

//   fn build_intrinsic_function(&mut self, name: String) {
//     match &name[..] {
//       "exit_with" => {
//         {
//           // add extern "exit" definition
//           let param_types = vec![self.llvm_context.i32_type().into()];
//           let param_types = param_types.as_slice();
//           let fn_type = self.llvm_context.void_type().fn_type(&param_types, false);

//           self
//             .llvm_module
//             .add_function("exit", fn_type, Some(Linkage::External));
//         }

//         // add wrapping func definition
//         let param_types = vec![self.llvm_context.i32_type().into()];
//         let param_types = param_types.as_slice();

//         let fn_type = self
//           .llvm_context
//           .struct_type(&[], false)
//           .fn_type(&param_types, false);

//         let function = self.llvm_module.add_function(&name[..], fn_type, None);

//         let entry = self.llvm_context.append_basic_block(function, "entry");
//         self.llvm_builder.position_at_end(entry);

//         let first_param = function.get_first_param().unwrap();
//         first_param.set_name("a");

//         // call extern function
//         let args = vec![first_param];
//         let func = self
//           .llvm_module
//           .get_function("exit")
//           .expect("should have function defined");
//         self.llvm_builder.build_call(func, args.as_slice(), "");

//         // return ()
//         let return_value = self.llvm_context.const_struct(&[], false);
//         self.llvm_builder.position_at_end(entry);
//         self.llvm_builder.build_return(Some(&return_value));
//       }

//       "print" => {
//         {
//           // add extern "puts" definition
//           let param_types = vec![self
//             .llvm_context
//             .i8_type()
//             .ptr_type(AddressSpace::Generic)
//             .into()];
//           let param_types = param_types.as_slice();
//           let fn_type = self.llvm_context.i32_type().fn_type(&param_types, false);

//           self
//             .llvm_module
//             .add_function("puts", fn_type, Some(Linkage::External));
//         }

//         // add definition for wrapping "print" func
//         let param_types = vec![self
//           .llvm_context
//           .i8_type()
//           .ptr_type(AddressSpace::Generic)
//           .into()];
//         let param_types = param_types.as_slice();

//         let fn_type = self
//           .llvm_context
//           .struct_type(&[], false)
//           .fn_type(&param_types, false);

//         let function = self.llvm_module.add_function(&name[..], fn_type, None);

//         let entry = self.llvm_context.append_basic_block(function, "entry");
//         self.llvm_builder.position_at_end(entry);

//         let first_param = function.get_first_param().unwrap();
//         first_param.set_name("a");

//         let args = vec![first_param];

//         let func = self
//           .llvm_module
//           .get_function("puts")
//           .expect("should have function defined");

//         self.llvm_builder.build_call(func, args.as_slice(), "");

//         let return_value = self.llvm_context.const_struct(&[], false);
//         self.llvm_builder.position_at_end(entry);
//         self.llvm_builder.build_return(Some(&return_value));
//       }

//       _ => unreachable!(),
//     }
//   }
// }

// impl<'ctx> Visitor for Emitter<'ctx> {
//   fn enter_module(&mut self, _node: &ModuleNode) {
//     let entry_block = self
//       .llvm_context
//       .append_basic_block(self.main_function, "entry");

//     self.llvm_builder.position_at_end(entry_block);
//   }

//   fn leave_module(&mut self, _node: &ModuleNode) {
//     let main_block = self.main_function.get_last_basic_block().unwrap();

//     self.llvm_builder.position_at_end(main_block);

//     let default_return_value = self.llvm_context.i32_type().const_int(47, true);

//     self.llvm_builder.build_return(Some(&default_return_value));
//   }

//   fn enter_top_level_statement(&mut self, node: &TopLevelStatementNode) {
//     match &node.kind {
//       TopLevelStatementKind::IntrinsicDef(def) => match &def.kind {
//         DefKind::Function { signature } => {
//           let name = signature.first().unwrap().0.name.clone();
//           self.build_intrinsic_function(name);
//         }

//         _ => {}
//       },

//       TopLevelStatementKind::Expr(expr) => {
//         // Go back to the end of the entry block in main
//         let block = self
//           .main_function
//           .get_last_basic_block()
//           .expect("should have at least one block");

//         self.llvm_builder.position_at_end(block);

//         self.compile_expr(expr);
//       }

//       _ => {}
//     }
//   }

//   fn enter_expr(&mut self, _node: &ExprNode) {}
// }
