#![allow(unused_imports)]

use crate::visitor::Visitor;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::passes::PassManager;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValue, BasicValueEnum, FloatValue, FunctionValue, PointerValue};
use inkwell::{FloatPredicate, OptimizationLevel};
use pluma_ast::nodes::*;

pub struct CodeGenerator<'a, 'ctx> {
  pub llvm_context: &'ctx Context,
  pub llvm_builder: &'a Builder<'ctx>,
  pub llvm_pass_manager: &'a PassManager<FunctionValue<'ctx>>,
  pub llvm_module: &'a Module<'ctx>,
  pub root_function: FunctionValue<'ctx>,
}

impl<'a, 'ctx> CodeGenerator<'a, 'ctx> {
  pub fn new(
    llvm_context: &'ctx Context,
    llvm_builder: &'a Builder<'ctx>,
    llvm_pass_manager: &'a PassManager<FunctionValue<'ctx>>,
    llvm_module: &'a Module<'ctx>,
  ) -> CodeGenerator<'a, 'ctx> {
    let return_type = llvm_context.f64_type().fn_type(&Vec::new(), false);

    let root_function = llvm_module.add_function("root", return_type, None);

    return CodeGenerator {
      llvm_context,
      llvm_builder,
      llvm_pass_manager,
      llvm_module,
      root_function,
    };
  }
}

impl<'a, 'ctx> Visitor for CodeGenerator<'a, 'ctx> {
  fn enter_module(&mut self, _node: &mut ModuleNode) {
    let entry_block = self
      .llvm_context
      .append_basic_block(self.root_function, "entry");

    self.llvm_builder.position_at_end(entry_block);
  }

  fn enter_top_level_statement(&mut self, node: &mut TopLevelStatementNode) {
    match &node.kind {
      TopLevelStatementKind::Expr(_expr) => {}

      _ => {}
    }
  }

  fn enter_expr(&mut self, _node: &mut ExprNode) {}
}
