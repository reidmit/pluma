#![allow(unused_must_use)]

use pluma_ast::*;
use pluma_visitor::*;
use std::fmt::Write;

pub struct TreeFormatter<W>
where
  W: Write,
{
  output: W,
}

impl<W> TreeFormatter<W>
where
  W: Write,
{
  pub fn new(output: W) -> Self {
    TreeFormatter { output }
  }

  fn out(&mut self, args: std::fmt::Arguments<'_>) {
    self.output.write_fmt(args);
  }

  fn line_break(&mut self) {
    self.output.write_char('\n');
  }

  fn format_expr(&mut self, node: &mut ExprNode) {
    match &mut node.kind {
      ExprKind::Tuple(entries) => {
        self.out(format_args!("("));

        let mut i = 0;
        let count = entries.len();

        for entry in entries {
          self.format_expr(entry);

          if i < count - 1 {
            self.out(format_args!(", "));
          }

          i += 1;
        }

        self.out(format_args!(")"));
      }

      ExprKind::Literal(lit) => self.format_literal(lit),

      _ => todo!("format other expr kinds"),
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
    self.out(format_args!("let "));
    self.format_pattern(&mut node.pattern);
    self.out(format_args!(" = "));
    self.format_expr(&mut node.value);
  }

  fn format_pattern(&mut self, node: &mut PatternNode) {
    match &node.kind {
      PatternKind::Identifier(id, is_mutable) => {
        if *is_mutable {
          self.out(format_args!("mut "));
        }

        self.out(format_args!("{}", id.name));
      }

      _ => todo!("format other pattern kinds"),
    }
  }
}

impl<W> Visitor for TreeFormatter<W>
where
  W: Write,
{
  fn enter_top_level_statement(&mut self, node: &mut TopLevelStatementNode) {
    match &mut node.kind {
      TopLevelStatementKind::Let(node) => self.format_let(node),
      _ => todo!("other top level kinds"),
    }

    self.line_break();
  }
}
