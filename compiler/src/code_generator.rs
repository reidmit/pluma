#![allow(unused_imports)]

use crate::diagnostics::Diagnostic;
use crate::visitor::Visitor;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::passes::PassManager;
use inkwell::targets::TargetTriple;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValue, BasicValueEnum, FloatValue, FunctionValue, PointerValue};
use inkwell::{FloatPredicate, OptimizationLevel};
use pluma_ast::nodes::*;
use std::io::prelude::*;
use std::process::{Command, Stdio};

pub struct CodeGenerator<'ctx> {
  llvm_context: &'ctx Context,
  llvm_builder: Builder<'ctx>,
  llvm_module: Module<'ctx>,
  main_function: FunctionValue<'ctx>,
}

impl<'ctx> CodeGenerator<'ctx> {
  pub fn new(llvm_context: &'ctx Context) -> CodeGenerator<'ctx> {
    let llvm_builder = llvm_context.create_builder();
    let llvm_module = llvm_context.create_module("root_module");

    llvm_module.set_triple(&TargetTriple::create("x86_64-apple-macosx10.15.0"));

    let return_type = llvm_context.i32_type().fn_type(&Vec::new(), false);
    let main_function = llvm_module.add_function("main", return_type, None);

    return CodeGenerator {
      llvm_context,
      llvm_builder,
      llvm_module,
      main_function,
    };
  }

  pub fn is_valid(&self) -> bool {
    // This will print error messages if things go wrong!
    self.main_function.verify(true)
  }

  pub fn write_to_string(&self) -> String {
    self.main_function.print_to_string().to_string()
  }

  pub fn write_to_path(&self, path: &std::path::Path) -> Result<(), Diagnostic> {
    let mut process = Command::new("clang")
      .args(&["-x", "ir", "-", "-o", path.to_str().unwrap()])
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .spawn()
      .map_err(|_| Diagnostic::error("Failed to spawn process."))?;

    let child_stdin = process.stdin.as_mut().unwrap();

    child_stdin
      .write_all(&self.write_to_string().as_bytes()[..])
      .map_err(|_| Diagnostic::error("Failed to pipe input to process."))?;

    let output = process
      .wait_with_output()
      .map_err(|_| Diagnostic::error("Problem waiting for process to exit."))?;

    if !output.status.success() {
      return Err(Diagnostic::error("Process did not terminate successfully."));
    }

    Ok(())
  }

  pub fn optimize(&mut self) {
    let llvm_pass_manager = PassManager::create(&self.llvm_module);
    // TODO: add passes!
    llvm_pass_manager.initialize();

    llvm_pass_manager.run_on(&mut self.main_function);
  }

  pub fn execute(&self) -> i32 {
    let execution_engine = self
      .llvm_module
      .create_jit_execution_engine(OptimizationLevel::None)
      .unwrap();

    let maybe_fn =
      unsafe { execution_engine.get_function::<unsafe extern "C" fn() -> i32>("main") };

    let compiled_fn = maybe_fn.expect("Should always have a function called 'main'");

    unsafe { compiled_fn.call() as i32 }
  }
}

impl<'ctx> Visitor for CodeGenerator<'ctx> {
  fn enter_module(&mut self, _node: &mut ModuleNode) {
    let entry_block = self
      .llvm_context
      .append_basic_block(self.main_function, "entry");

    self.llvm_builder.position_at_end(entry_block);

    let default_return_value = self.llvm_context.i32_type().const_int(47, true);

    self.llvm_builder.build_return(Some(&default_return_value));
  }

  fn enter_top_level_statement(&mut self, node: &mut TopLevelStatementNode) {
    match &node.kind {
      TopLevelStatementKind::Expr(_expr) => {}

      _ => {}
    }
  }

  fn enter_expr(&mut self, _node: &mut ExprNode) {}
}
