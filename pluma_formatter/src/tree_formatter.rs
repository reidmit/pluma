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

  fn format_identifier(&mut self, node: &mut IdentifierNode) {
    self.out(format_args!("{}", node.name));
  }

  fn format_call(&mut self, node: &mut CallNode) {
    match &mut node.callee.kind {
      ExprKind::MultiPartIdentifier(parts) => {
        let count = parts.len();
        let mut i = 0;

        while i < count {
          let mut part_name = parts.get_mut(i).unwrap();
          let mut part_arg = node.args.get_mut(i).unwrap();

          if i > 0 {
            self.out_str(" ");
          }
          self.format_identifier(&mut part_name);
          self.out_str(" ");
          self.format_expr(&mut part_arg);

          i += 1;
        }
      }

      _ => {
        self.format_expr(&mut node.callee);

        for arg in &mut node.args {
          self.out_str(" ");
          self.format_expr(arg);
        }
      }
    }
  }

  fn format_expr(&mut self, node: &mut ExprNode) {
    match &mut node.kind {
      ExprKind::Block { params, body } => {
        self.brace_depth += 1;

        self.out_str("{");

        let one_line = body.len() < 2;

        if !params.is_empty() {
          let count = params.len();
          let mut i = 0;

          for param in params {
            self.out_str(" ");

            self.format_identifier(param);

            if i < count - 1 {
              self.out_str(",");
            }

            i += 1;
          }

          self.out_str(" =>");
        }

        if one_line {
          for stmt in body {
            self.format_statement(stmt);
          }
        } else {
          for stmt in body {
            self.line_break();
            self.format_statement(stmt);
          }

          self.line_break();
        }

        self.brace_depth -= 1;

        self.out_str("}");
      }

      ExprKind::Call(call) => self.format_call(call),

      ExprKind::Identifier(ident) => self.format_identifier(ident),

      ExprKind::UnlabeledTuple(entries) => {
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

      ExprKind::Literal(lit) => self.format_literal(lit),

      _o => todo!("format other expr kinds"),
    }
  }

  fn format_literal(&mut self, node: &mut LiteralNode) {
    match &mut node.kind {
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

  fn format_let(&mut self, node: &mut LetNode) {
    self.out_str("let ");
    self.format_pattern(&mut node.pattern);
    self.out_str(" = ");
    self.format_expr(&mut node.value);
  }

  fn format_pattern(&mut self, node: &mut PatternNode) {
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

  fn format_statement(&mut self, node: &mut StatementNode) {
    for _ in 0..self.brace_depth {
      self.out_str("  ");
    }

    match &mut node.kind {
      StatementKind::Expr(expr) => self.format_expr(expr),
      StatementKind::Let(let_node) => self.format_let(let_node),
      _ => todo!("other stmt kinds"),
    }
  }
}

impl<W> Visitor for TreeFormatter<W>
where
  W: Write,
{
  fn enter_top_level_statement(&mut self, node: &mut TopLevelStatementNode) {
    match &mut node.kind {
      TopLevelStatementKind::Expr(node) => self.format_expr(node),
      TopLevelStatementKind::Let(node) => self.format_let(node),
      _ => todo!("other top level kinds"),
    }

    self.line_break();
  }
}
