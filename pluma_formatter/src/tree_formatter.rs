#![allow(unused_must_use)]

use pluma_ast::*;
use pluma_visitor::*;
use std::fmt::Write;

pub struct TreeFormatter<W>
where
  W: Write,
{
  output: W,
  brace_depth: usize,
}

impl<W> TreeFormatter<W>
where
  W: Write,
{
  pub fn new(output: W) -> Self {
    TreeFormatter {
      output,
      brace_depth: 0,
    }
  }

  fn out(&mut self, args: std::fmt::Arguments<'_>) {
    self.output.write_fmt(args);
  }

  fn out_str(&mut self, s: &str) {
    self.output.write_str(s);
  }

  fn line_break(&mut self) {
    self.output.write_char('\n');
  }

  fn format_identifier(&mut self, node: &IdentifierNode) {
    self.out(format_args!("{}", node.name));
  }

  fn format_block(&mut self, node: &BlockNode) {
    self.brace_depth += 1;

    self.out_str("{");

    let one_line = node.body.len() < 2;

    if !node.params.is_empty() {
      let count = &node.params.len();
      let mut i = 0;

      for param in &node.params {
        self.out_str(" ");

        self.format_pattern(param);

        if i < count - 1 {
          self.out_str(",");
        }

        i += 1;
      }

      self.out_str(" =>");
    }

    if one_line {
      for stmt in &node.body {
        self.format_statement(stmt);
      }
    } else {
      for stmt in &node.body {
        self.line_break();
        self.format_statement(stmt);
      }

      self.line_break();
    }

    self.brace_depth -= 1;

    self.out_str("}");
  }

  fn format_call(&mut self, node: &CallNode) {
    // match &node.callee.kind {
    //   ExprKind::MultiPartIdentifier { parts } => {
    //     let count = parts.len();
    //     let mut i = 0;

    //     while i < count {
    //       let part_name = parts.get(i).unwrap();
    //       let part_arg = node.args.get(i).unwrap();

    //       if i > 0 {
    //         self.out_str(" ");
    //       }
    //       self.format_identifier(&part_name);
    //       self.out_str(" ");
    //       self.format_expr(&part_arg);

    //       i += 1;
    //     }
    //   }

    //   _ => {
    //     self.format_expr(&node.callee);

    //     for arg in &node.args {
    //       self.out_str(" ");
    //       self.format_expr(arg);
    //     }
    //   }
    // }
  }

  fn format_expr(&mut self, node: &ExprNode) {
    match &node.kind {
      ExprKind::Block { block } => self.format_block(block),

      ExprKind::Call { call } => self.format_call(call),

      ExprKind::Identifier { ident } => self.format_identifier(ident),

      ExprKind::UnlabeledTuple { entries } => {
        self.out_str("(");

        let mut i = 0;
        let count = entries.len();

        for entry in entries {
          self.format_expr(entry);

          if i < count - 1 {
            self.out_str(", ");
          }

          i += 1;
        }

        self.out_str(")");
      }

      ExprKind::Literal { literal } => self.format_literal(literal),

      _o => todo!("format other expr kinds"),
    }
  }

  fn format_literal(&mut self, node: &LiteralNode) {
    match &node.kind {
      LiteralKind::FloatDecimal(val) => {
        self.out(format_args!("{}", val));
      }
      LiteralKind::IntDecimal(val) => {
        self.out(format_args!("{}", val));
      }
      LiteralKind::IntOctal(val) => {
        self.out(format_args!("{}", val));
      }
      LiteralKind::IntHex(val) => {
        self.out(format_args!("{}", val));
      }
      LiteralKind::IntBinary(val) => {
        self.out(format_args!("{}", val));
      }
      LiteralKind::Str(val) => {
        self.out(format_args!("\"{}\"", val));
      }
    }
  }

  fn format_let(&mut self, node: &LetNode) {
    self.out_str("let ");
    self.format_pattern(&node.pattern);
    self.out_str(" = ");
    self.format_expr(&node.value);
  }

  fn format_pattern(&mut self, node: &PatternNode) {
    match &node.kind {
      PatternKind::Identifier(id, is_mutable) => {
        if *is_mutable {
          self.out_str("mut ");
        }

        self.out(format_args!("{}", id.name));
      }

      _ => todo!("format other pattern kinds"),
    }
  }

  fn format_statement(&mut self, node: &StatementNode) {
    for _ in 0..self.brace_depth {
      self.out_str("  ");
    }

    match &node.kind {
      StatementKind::Expr(expr) => self.format_expr(expr),
      StatementKind::Let(let_node) => self.format_let(let_node),
    }
  }
}

impl<W> Visitor for TreeFormatter<W>
where
  W: Write,
{
  fn enter_top_level_statement(&mut self, node: &TopLevelStatementNode) {
    match &node.kind {
      TopLevelStatementKind::Expr(node) => self.format_expr(node),
      TopLevelStatementKind::Let(node) => self.format_let(node),
      _ => todo!("other top level kinds"),
    }

    self.line_break();
  }
}
