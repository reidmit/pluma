use crate::parse_error::*;
use crate::tokenizer::Tokenizer;
use crate::tokens::Token;
use pluma_ast::*;
use std::collections::HashMap;

macro_rules! current_token_is {
  ($self:ident, $tokType:path) => {
    match $self.current_token {
      Some($tokType(..)) => true,
      _ => false,
    }
  };
}

macro_rules! next_token_is {
  ($self:ident, $tokType:path) => {
    match $self.tokenizer.peek() {
      Some($tokType(..)) => true,
      _ => false,
    }
  };
}

macro_rules! expect_token_and_do {
  ($self:ident, $tokType:path, $block:tt) => {
    match $self.current_token {
      Some($tokType(..)) => $block,
      Some(tok) => {
        return $self.error(ParseError {
          pos: tok.get_position(),
          kind: ParseErrorKind::UnexpectedToken($tokType(0, 0)),
        });
      }
      None => {
        return $self.error(ParseError {
          pos: ($self.source.len(), $self.source.len()),
          kind: ParseErrorKind::UnexpectedEOF($tokType(0, 0)),
        })
      }
    }
  };
}

macro_rules! read_string {
  ($self:ident, $start:expr, $end:expr) => {
    String::from_utf8($self.source[$start..$end].to_vec()).expect("not utf-8");
  };
}

macro_rules! read_string_with_escapes {
  ($self:ident, $start:expr, $end:expr) => {
    read_string!($self, $start, $end)
      .replace("\\\"", "\"")
      .replace("\\\\", "\\")
      .replace("\\t", "\t")
      .replace("\\r", "\r")
      .replace("\\n", "\n");
  };
}

pub struct Parser<'a> {
  source: &'a Vec<u8>,
  tokenizer: Tokenizer<'a>,
  index: usize,
  errors: Vec<ParseError>,
  current_token: Option<Token>,
  prev_token: Option<Token>,
  current_visibility: ExportVisibility,
  line_break_starts: Vec<usize>,
}

impl<'a> Parser<'a> {
  pub fn new(source: &'a Vec<u8>, tokenizer: Tokenizer<'a>) -> Parser<'a> {
    return Parser {
      source,
      tokenizer,
      index: 0,
      errors: Vec::new(),
      current_token: None,
      prev_token: None,
      current_visibility: ExportVisibility::Public,
      line_break_starts: Vec::new(),
    };
  }

  pub fn parse_module(
    &mut self,
  ) -> (
    ModuleNode,
    Vec<UseNode>,
    (HashMap<usize, String>, Vec<usize>),
    Vec<ParseError>,
  ) {
    let mut imports = Vec::new();
    let mut body = Vec::new();

    // Read the first token
    self.advance();

    loop {
      self.skip_line_breaks();

      if !current_token_is!(self, Token::KeywordUse) {
        break;
      }

      match self.parse_use_statement() {
        Some(use_node) => imports.push(use_node),
        _ => break,
      }
    }

    loop {
      self.skip_line_breaks();

      match self.parse_top_level_statement() {
        Some(statement) => body.push(statement),
        _ => break,
      }
    }

    if let Some(_extra_token) = self.current_token {
      self.error::<ModuleNode>(ParseError {
        pos: self.current_token_position(),
        kind: ParseErrorKind::UnexpectedTokenExpectedEOF,
      });
    }

    let start = body.first().map_or(0, |node| node.pos.0);
    let end = body.last().map_or(0, |node| node.pos.1);

    let module_node = ModuleNode {
      pos: (start, end),
      body,
    };

    let comment_data = (
      self.tokenizer.comments.clone(),
      self.line_break_starts.clone(),
    );

    (module_node, imports, comment_data, self.errors.clone())
  }

  fn advance(&mut self) {
    self.prev_token = self.current_token;
    self.current_token = self.tokenizer.next();
    self.index += 1;
  }

  fn skip_line_breaks(&mut self) -> bool {
    let mut skipped_any = false;

    while current_token_is!(self, Token::LineBreak) {
      self.line_break_starts.push(self.current_token_position().0);

      skipped_any = true;

      self.advance()
    }

    skipped_any
  }

  fn current_token_position(&self) -> (usize, usize) {
    match self.current_token {
      Some(token) => token.get_position(),
      _ => match self.prev_token {
        Some(token) => token.get_position(),
        _ => (0, 0),
      },
    }
  }

  fn error<A>(&mut self, err: ParseError) -> Option<A> {
    self.errors.push(err);
    None
  }

  fn parse_alias(&mut self) -> Option<TypeDefNode> {
    let start = expect_token_and_do!(self, Token::KeywordAlias, {
      let pos = self.current_token_position();
      self.advance();
      pos.0
    });

    let name = match self.parse_type_identifier() {
      Some(type_id) => type_id,
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingTypeNameInTypeDefinition,
        })
      }
    };

    self.skip_line_breaks();

    let generic_type_constraints = self.parse_generic_type_constraints().unwrap_or_default();

    let type_expr = match self.parse_type_expression() {
      Some(expr) => expr,
      _ => {
        // Assume that the failure to parse the type expression
        // has already generated an error.
        return None;
      }
    };

    Some(TypeDefNode {
      pos: (start, type_expr.pos.1),
      kind: TypeDefKind::Alias { of: type_expr },
      name,
      generic_type_constraints,
      visibility: self.current_visibility,
    })
  }

  fn parse_binary_number(&mut self) -> Option<LiteralNode> {
    let (start, end, value) = expect_token_and_do!(self, Token::BinaryDigits, {
      let (start, end) = self.current_token_position();
      (start, end, self.parse_numeric_literal(start + 2, end, 2))
    });

    self.advance();

    Some(LiteralNode {
      kind: LiteralKind::IntBinary(value),
      pos: (start, end),
    })
  }

  fn parse_block(&mut self) -> Option<BlockNode> {
    let block_start = expect_token_and_do!(self, Token::LeftBrace, {
      let (start, _) = self.current_token_position();
      self.advance();
      start
    });

    self.skip_line_breaks();

    let mut params = Vec::new();
    let mut body = Vec::new();

    if current_token_is!(self, Token::Pipe) {
      self.advance();

      while let Some(pattern) = self.parse_pattern() {
        params.push(pattern);

        match self.current_token {
          Some(Token::Comma(..)) => self.advance(),
          Some(Token::Pipe(..)) => {
            break;
          }
          _ => break,
        }
      }

      expect_token_and_do!(self, Token::Pipe, {
        self.advance();
      });
    }

    self.skip_line_breaks();

    while let Some(node) = self.parse_statement() {
      body.push(node);

      self.skip_line_breaks();
    }

    self.skip_line_breaks();

    let block_end = expect_token_and_do!(self, Token::RightBrace, {
      let pos = self.current_token_position();
      self.advance();
      pos.1
    });

    Some(BlockNode {
      pos: (block_start, block_end),
      params,
      body,
    })
  }

  fn parse_call_with_receiver(&mut self, receiver: ExprNode) -> Option<ExprNode> {
    let mut method_parts = Vec::new();
    let mut args = Vec::new();

    while current_token_is!(self, Token::Identifier) {
      match self.parse_identifier(false) {
        Some(next_callee_part) => {
          method_parts.push(next_callee_part);

          // Grab the argument for this part
          match self.parse_expression_precedence_2() {
            Some(arg) => args.push(arg),
            _ => {
              // If there's no argument, break out of the loop. No more parts allowed.
              // Only the last part may not take an argument.
              // ex: thing . part1 "arg1" part2
              break;
            }
          }
        }

        _ => {
          return self.error(ParseError {
            pos: self.current_token_position(),
            kind: ParseErrorKind::UnexpectedToken(Token::Identifier(0, 0)),
          })
        }
      }
    }

    let start = receiver.pos.0;

    if args.is_empty() {
      // If we collected 0 args, it's an expression like `thing.field`. Consider a plain
      // Access, not a call (even though it may be calling a method with no args). The analyzer
      // will figure that out later.
      let ident = method_parts[0].clone();

      let property = ExprNode {
        pos: ident.pos,
        kind: ExprKind::Identifier { ident },
        typ: ValueType::Unknown,
      };

      return Some(ExprNode {
        pos: (start, property.pos.1),
        kind: ExprKind::Access {
          receiver: Box::new(receiver),
          property: Box::new(property),
        },
        typ: ValueType::Unknown,
      });
    }

    let end = args.last().unwrap().pos.1;

    let property = if method_parts.len() == 1 {
      let ident = method_parts[0].clone();

      ExprNode {
        pos: ident.pos,
        kind: ExprKind::Identifier { ident },
        typ: ValueType::Unknown,
      }
    } else {
      let multi_start = method_parts.first().unwrap().pos.0;
      let multi_end = method_parts.last().unwrap().pos.1;

      ExprNode {
        pos: (multi_start, multi_end),
        kind: ExprKind::MultiPartIdentifier {
          parts: method_parts,
        },
        typ: ValueType::Unknown,
      }
    };

    let callee = ExprNode {
      pos: (start, property.pos.1),
      kind: ExprKind::Access {
        receiver: Box::new(receiver),
        property: Box::new(property),
      },
      typ: ValueType::Unknown,
    };

    let call = CallNode {
      pos: (start, end),
      callee: Box::new(callee),
      args,
      typ: ValueType::Unknown,
    };

    Some(ExprNode {
      pos: call.pos,
      kind: ExprKind::Call { call },
      typ: ValueType::Unknown,
    })
  }

  fn parse_call_without_receiver(&mut self, prev_expr: ExprNode) -> Option<CallNode> {
    let first_arg = self.parse_expression_precedence_2()?;
    let first_arg_end = first_arg.pos.1;

    let allow_multi_part = match prev_expr.kind {
      ExprKind::Identifier { .. } => true,
      ExprKind::QualifiedIdentifier { .. } => true,
      _ => false,
    };

    let mut args = vec![first_arg];

    if !allow_multi_part {
      // Simpler case: can't have a multi-part identifier here, so our single arg
      // must be the only arg.

      let start = prev_expr.pos.0;

      match self.current_token {
        Some(token) if token.can_start_expression() => {
          self.error::<ExprNode>(ParseError {
            pos: self.current_token_position(),
            kind: ParseErrorKind::UnexpectedExpressionAfterCall,
          });
        }
        _ => {}
      }

      return Some(CallNode {
        pos: (start, first_arg_end),
        callee: Box::new(prev_expr),
        args,
        typ: ValueType::Unknown,
      });
    }

    let mut callee_parts = Vec::new();
    let mut first_part_qualifier = None;

    match prev_expr.kind {
      ExprKind::Identifier { ident } => callee_parts.push(ident),
      ExprKind::QualifiedIdentifier { ident, qualifier } => {
        first_part_qualifier = Some(qualifier);
        callee_parts.push(*ident);
      }
      _ => unreachable!(),
    };

    while current_token_is!(self, Token::Identifier) {
      match self.parse_identifier(false) {
        Some(next_callee_part) => {
          callee_parts.push(next_callee_part);

          // Grab the argument for this part
          match self.parse_expression_precedence_2() {
            Some(arg) => args.push(arg),
            _ => {
              return self.error(ParseError {
                pos: self.current_token_position(),
                kind: ParseErrorKind::MissingArgumentInCall,
              })
            }
          }
        }

        _ => {
          return self.error(ParseError {
            pos: self.current_token_position(),
            kind: ParseErrorKind::UnexpectedToken(Token::Identifier(0, 0)),
          })
        }
      }
    }

    match self.current_token {
      Some(token) if token.can_start_expression() => {
        self.error::<ExprNode>(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::UnexpectedExpressionAfterCall,
        });
      }
      _ => {}
    }

    let callee_start = if let Some(qualifier) = &first_part_qualifier {
      qualifier.pos.0
    } else {
      callee_parts.first().unwrap().pos.0
    };
    let callee_end = callee_parts.last().unwrap().pos.1;
    let call_end = if args.is_empty() {
      callee_end
    } else {
      args.last().unwrap().pos.1
    };

    let callee_kind = match (callee_parts.len() == 1, first_part_qualifier) {
      (true, Some(qualifier)) => {
        let ident = callee_parts[0].clone();
        ExprKind::QualifiedIdentifier {
          qualifier,
          ident: Box::new(ident),
        }
      }
      (true, None) => {
        let ident = callee_parts[0].clone();
        ExprKind::Identifier { ident }
      }
      (false, Some(qualifier)) => ExprKind::QualifiedMultiPartIdentifier {
        qualifier,
        parts: callee_parts,
      },
      (false, None) => ExprKind::MultiPartIdentifier {
        parts: callee_parts,
      },
    };

    Some(CallNode {
      pos: (callee_start, call_end),
      callee: Box::new(ExprNode {
        pos: (callee_start, callee_end),
        kind: callee_kind,
        typ: ValueType::Unknown,
      }),
      args,
      typ: ValueType::Unknown,
    })
  }

  fn parse_decimal_number(&mut self) -> Option<LiteralNode> {
    let (start, end) = expect_token_and_do!(self, Token::DecimalDigits, {
      let pos = self.current_token_position();
      self.advance();
      pos
    });

    if current_token_is!(self, Token::Dot) && next_token_is!(self, Token::DecimalDigits) {
      self.advance();

      expect_token_and_do!(self, Token::DecimalDigits, {
        let (_, end) = self.current_token_position();

        self.advance();

        let str_value = read_string!(self, start, end);
        let float_value = str_value.parse::<f64>().unwrap();

        return Some(LiteralNode {
          kind: LiteralKind::FloatDecimal(float_value),
          pos: (start, end),
        });
      });
    }

    let value = self.parse_numeric_literal(start, end, 10);

    Some(LiteralNode {
      kind: LiteralKind::IntDecimal(value),
      pos: (start, end),
    })
  }

  fn parse_intrinsic_definition(&mut self) -> Option<IntrinsicDefNode> {
    let start = expect_token_and_do!(self, Token::KeywordIntrinsicDef, {
      let pos = self.current_token_position();
      self.advance();
      pos.0
    });

    let kind = match self.parse_definition_kind() {
      Some(kind_node) => kind_node,
      _ => {
        // Just return without adding a new error. Assumes that the
        // failure to parse the kind has already generated an error.
        return None;
      }
    };

    let return_type = if current_token_is!(self, Token::Arrow) {
      self.advance();

      match self.parse_type_expression() {
        Some(type_node) => Some(type_node),
        _ => {
          return self.error(ParseError {
            pos: self.current_token_position(),
            kind: ParseErrorKind::IncompleteMethodSignature,
          })
        }
      }
    } else {
      None
    };

    self.skip_line_breaks();

    let generic_type_constraints = self.parse_generic_type_constraints().unwrap_or_default();

    let end = match self.prev_token {
      Some(token) => token.get_position().1,
      _ => start,
    };

    Some(IntrinsicDefNode {
      pos: (start, end),
      visibility: self.current_visibility,
      kind,
      return_type,
      generic_type_constraints,
    })
  }

  fn parse_definition(&mut self) -> Option<DefNode> {
    let start = expect_token_and_do!(self, Token::KeywordDef, {
      let pos = self.current_token_position();
      self.advance();
      pos.0
    });

    let kind = match self.parse_definition_kind() {
      Some(kind_node) => kind_node,
      _ => {
        // Just return without adding a new error. Assumes that the
        // failure to parse the kind has already generated an error.
        return None;
      }
    };

    let return_type = if current_token_is!(self, Token::Arrow) {
      self.advance();

      match self.parse_type_expression() {
        Some(type_node) => Some(type_node),
        _ => {
          return self.error(ParseError {
            pos: self.current_token_position(),
            kind: ParseErrorKind::MissingReturnType,
          })
        }
      }
    } else {
      None
    };

    self.skip_line_breaks();

    let generic_type_constraints = self.parse_generic_type_constraints().unwrap_or_default();

    let block = match self.parse_block() {
      Some(block) => block,
      _ => return None,
    };

    Some(DefNode {
      pos: (start, block.pos.1),
      visibility: self.current_visibility,
      kind,
      return_type,
      generic_type_constraints,
      block,
    })
  }

  fn parse_generic_type_constraints(&mut self) -> Option<GenericTypeConstraints> {
    let mut generic_type_constraints = Vec::new();

    if current_token_is!(self, Token::KeywordWhere) {
      self.advance();

      while let Some(generic_name) = self.parse_identifier(false) {
        expect_token_and_do!(self, Token::DoubleColon, {
          self.advance();
        });

        let type_expr = match self.parse_type_identifier() {
          Some(expr) => expr,
          _ => {
            return self.error(ParseError {
              pos: self.current_token_position(),
              kind: ParseErrorKind::MissingType,
            });
          }
        };

        generic_type_constraints.push((generic_name, type_expr));

        match self.current_token {
          Some(Token::Comma(..)) => self.advance(),
          _ => break,
        }
      }
    }

    self.skip_line_breaks();

    Some(generic_type_constraints)
  }

  fn parse_definition_kind(&mut self) -> Option<DefKind> {
    // The first ident might be a type ident or a simple method part name
    let type_ident = match self.parse_type_identifier() {
      Some(t) => t,
      None => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::IncompleteMethodSignature,
        })
      }
    };

    let mut receiver = None;
    let mut signature: Signature = Vec::new();

    if current_token_is!(self, Token::Dot) {
      // If we have a dot now, we know the first ident was a receiver type
      receiver = Some(type_ident);
      self.advance();
    } else {
      // If not, the first ident was the first part of the method name
      // So, grab the param type for this part
      match self.parse_type_expression() {
        Some(part_param) => {
          let ident = IdentifierNode {
            pos: type_ident.pos,
            name: type_ident.name,
          };

          signature.push((ident, Box::new(part_param)))
        }
        _ => {
          return self.error(ParseError {
            pos: self.current_token_position(),
            kind: ParseErrorKind::IncompleteMethodSignature,
          })
        }
      }
    }

    // Now, collect any remaining parts
    while current_token_is!(self, Token::Identifier) {
      let part_name = self.parse_identifier(false).unwrap();

      match self.parse_type_expression() {
        Some(part_param) => signature.push((part_name, Box::new(part_param))),
        _ => {
          return self.error(ParseError {
            pos: self.current_token_position(),
            kind: ParseErrorKind::IncompleteMethodSignature,
          })
        }
      }
    }

    if signature.is_empty() {
      return self.error(ParseError {
        pos: self.current_token_position(),
        kind: ParseErrorKind::IncompleteMethodSignature,
      });
    }

    if let Some(rec) = receiver {
      return Some(DefKind::Method {
        receiver: Box::new(rec),
        signature,
      });
    }

    Some(DefKind::Function { signature })
  }

  fn parse_enum(&mut self) -> Option<TypeDefNode> {
    let start = expect_token_and_do!(self, Token::KeywordEnum, {
      let pos = self.current_token_position();
      self.advance();
      pos.0
    });

    let name = match self.parse_type_identifier() {
      Some(type_id) => type_id,
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingTypeNameInTypeDefinition,
        })
      }
    };

    self.skip_line_breaks();

    let generic_type_constraints = self.parse_generic_type_constraints().unwrap_or_default();

    let mut variants = Vec::new();

    expect_token_and_do!(self, Token::Pipe, {});

    while let Some(Token::Pipe(..)) = self.current_token {
      self.advance();

      match self.parse_identifier(false) {
        Some(id) => {
          match self.current_token {
            // A variant can either be a call with an argument, in which case we
            // expect to find an argument here:
            Some(Token::Identifier(..)) | Some(Token::LeftParen(..)) => {
              match self.parse_type_expression() {
                Some(type_expr) => variants.push(EnumVariantNode {
                  pos: (id.pos.0, type_expr.pos.1),
                  kind: EnumVariantKind::Constructor(id, type_expr),
                }),
                _ => return None,
              }
            }

            // ...or else just a plain identifier:
            _ => variants.push(EnumVariantNode {
              pos: id.pos,
              kind: EnumVariantKind::Identifier(id),
            }),
          }
        }
        _ => return None,
      }

      self.skip_line_breaks();
    }

    if variants.is_empty() {
      return self.error(ParseError {
        pos: self.current_token_position(),
        kind: ParseErrorKind::MissingEnumValues,
      });
    }

    Some(TypeDefNode {
      pos: (start, variants.last().unwrap().pos.1),
      visibility: self.current_visibility,
      kind: TypeDefKind::Enum { variants },
      name,
      generic_type_constraints,
    })
  }

  /// This method and the following methods (...precedence_<n>) parse
  /// expressions according to their precedence. The "top level", handled
  /// in this method, represents the lowest (weakest) precedence: assignment
  /// via the = operator.
  fn parse_expression(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_1();

    while expr.is_some() {
      match self.current_token {
        Some(Token::Equal(..)) => {
          self.advance();

          let left_side = expr.unwrap();
          let right_side = self.parse_expression_precedence_1().unwrap();
          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::Assignment {
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_1(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_2();

    while expr.is_some() {
      // Skip line breaks, but keep track of whether there were actually any to skip.
      // We need to know this because line breaks are allowed before a '.' in a chain
      // expression (e.g. "a\n .b\n .c"), but not between arguments in a function call
      // (e.g. "something someArg\nthisIsNotAnArg").
      let had_line_breaks = self.skip_line_breaks();

      match self.current_token {
        Some(Token::Dot(..)) => {
          self.advance();

          let receiver = expr.unwrap();

          expr = self.parse_call_with_receiver(receiver);
        }

        // Another token on this same line?! Maybe it's an argument in call expression
        Some(other_token) if !had_line_breaks && other_token.can_start_expression() => {
          let prev_expr = expr.unwrap();
          let call_node = self.parse_call_without_receiver(prev_expr)?;

          expr = Some(ExprNode {
            pos: call_node.pos,
            kind: ExprKind::Call { call: call_node },
            typ: ValueType::Unknown,
          });
        }

        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_2(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_3();

    while expr.is_some() {
      match self.current_token {
        Some(Token::DoublePipe(..)) => {
          let op_node = self.parse_operator().unwrap();
          let left_side = expr.unwrap();
          let right_side = self.parse_expression_precedence_3().unwrap();
          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::BinaryOperation {
              op: Box::new(op_node),
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_3(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_4();

    while expr.is_some() {
      match self.current_token {
        Some(Token::DoubleAnd(..)) => {
          let op_node = self.parse_operator().unwrap();
          let left_side = expr.unwrap();
          let right_side = self.parse_expression_precedence_4().unwrap();
          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::BinaryOperation {
              op: Box::new(op_node),
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_4(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_5();

    while expr.is_some() {
      match self.current_token {
        Some(Token::DoubleColon(start, _)) => {
          self.advance();
          let left_side = expr.unwrap();
          let right_side = self.parse_type_expression().unwrap();
          let end = right_side.pos.1;

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::TypeAssertion {
              expr: Box::new(left_side),
              asserted_type: right_side,
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_5(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_6();

    while expr.is_some() {
      match self.current_token {
        Some(Token::Pipe(..)) => {
          let op_node = self.parse_operator().unwrap();
          let left_side = expr.unwrap();
          let right_side = self.parse_expression_precedence_6().unwrap();
          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::BinaryOperation {
              op: Box::new(op_node),
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_6(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_7();

    while expr.is_some() {
      match self.current_token {
        Some(Token::Caret(..)) => {
          let op_node = self.parse_operator().unwrap();
          let left_side = expr.unwrap();
          let right_side = self.parse_expression_precedence_7().unwrap();
          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::BinaryOperation {
              op: Box::new(op_node),
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_7(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_8();

    while expr.is_some() {
      match self.current_token {
        Some(Token::And(..)) => {
          let op_node = self.parse_operator().unwrap();
          let left_side = expr.unwrap();
          let right_side = self.parse_expression_precedence_8().unwrap();
          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::BinaryOperation {
              op: Box::new(op_node),
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_8(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_9();

    while expr.is_some() {
      match self.current_token {
        Some(Token::DoubleEqual(..)) | Some(Token::BangEqual(..)) => {
          let op_node = self.parse_operator().unwrap();
          let left_side = expr.unwrap();
          let right_side = self
            .parse_expression_precedence_9()
            .expect("expr after == or !=");

          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::BinaryOperation {
              op: Box::new(op_node),
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_9(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_10();

    while expr.is_some() {
      match self.current_token {
        Some(Token::LeftAngle(..))
        | Some(Token::RightAngle(..))
        | Some(Token::LeftAngleEqual(..))
        | Some(Token::RightAngleEqual(..)) => {
          let op_node = self.parse_operator().unwrap();
          let left_side = expr.unwrap();
          let right_side = self.parse_expression_precedence_10().unwrap();
          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::BinaryOperation {
              op: Box::new(op_node),
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_10(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_11();

    while expr.is_some() {
      match self.current_token {
        Some(Token::DoubleLeftAngle(..)) | Some(Token::DoubleRightAngle(..)) => {
          let op_node = self.parse_operator().unwrap();
          let left_side = expr.unwrap();
          let right_side = self.parse_expression_precedence_11().unwrap();
          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::BinaryOperation {
              op: Box::new(op_node),
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_11(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_12();

    while expr.is_some() {
      match self.current_token {
        Some(Token::Plus(..)) | Some(Token::Minus(..)) | Some(Token::DoublePlus(..)) => {
          let op_node = self.parse_operator().unwrap();
          let left_side = expr.unwrap();
          let right_side = self.parse_expression_precedence_12()?;
          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::BinaryOperation {
              op: Box::new(op_node),
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_12(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_13();

    while expr.is_some() {
      match self.current_token {
        Some(Token::Star(..)) | Some(Token::ForwardSlash(..)) | Some(Token::Percent(..)) => {
          let op_node = self.parse_operator().unwrap();
          let left_side = expr.unwrap();
          let right_side = self.parse_expression_precedence_13().unwrap();
          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::BinaryOperation {
              op: Box::new(op_node),
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_13(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_expression_precedence_14();

    while expr.is_some() {
      match self.current_token {
        Some(Token::DoubleStar(..)) => {
          let op_node = self.parse_operator().unwrap();
          let left_side = expr.unwrap();
          let right_side = self.parse_expression_precedence_14().unwrap();
          let (start, end) = (left_side.pos.0, right_side.pos.1);

          expr = Some(ExprNode {
            pos: (start, end),
            kind: ExprKind::BinaryOperation {
              op: Box::new(op_node),
              left: Box::new(left_side),
              right: Box::new(right_side),
            },
            typ: ValueType::Unknown,
          });
        }
        _ => break,
      }
    }

    expr
  }

  fn parse_expression_precedence_14(&mut self) -> Option<ExprNode> {
    match self.current_token {
      Some(Token::Bang(..)) | Some(Token::Minus(..)) | Some(Token::Tilde(..)) => {
        let op_node = self.parse_operator().unwrap();
        let right_side = self.parse_expression_precedence_14().unwrap();
        let (start, end) = (op_node.pos.0, right_side.pos.1);

        Some(ExprNode {
          pos: (start, end),
          kind: ExprKind::UnaryOperation {
            op: Box::new(op_node),
            right: Box::new(right_side),
          },
          typ: ValueType::Unknown,
        })
      }

      _ => self.parse_term(),
    }
  }

  fn parse_hex_number(&mut self) -> Option<LiteralNode> {
    let (start, end, value) = expect_token_and_do!(self, Token::HexDigits, {
      let (start, end) = self.current_token_position();
      (start, end, self.parse_numeric_literal(start + 2, end, 16))
    });

    self.advance();

    Some(LiteralNode {
      kind: LiteralKind::IntHex(value),
      pos: (start, end),
    })
  }

  fn parse_identifier(&mut self, allow_special: bool) -> Option<IdentifierNode> {
    let (start, end) = match self.current_token {
      Some(Token::Identifier(start, end)) => {
        self.advance();
        (start, end)
      }
      Some(Token::IdentifierSpecialOther(start, end)) if allow_special => {
        self.advance();
        (start, end)
      }
      Some(Token::IdentifierSpecialParam(start, end)) if allow_special => {
        self.advance();
        (start, end)
      }
      _ => return None,
    };

    let name = read_string!(self, start, end);

    Some(IdentifierNode {
      pos: (start, end),
      name,
    })
  }

  fn parse_internal(&mut self) -> Option<TopLevelStatementNode> {
    let pos = expect_token_and_do!(self, Token::KeywordInternal, {
      let pos = self.current_token_position();
      self.advance();
      pos
    });

    self.current_visibility = ExportVisibility::Internal;

    Some(TopLevelStatementNode {
      pos: pos,
      kind: TopLevelStatementKind::VisibilityMarker(ExportVisibility::Internal),
    })
  }

  fn parse_intrinsic_type(&mut self) -> Option<IntrinsicTypeDefNode> {
    let start = expect_token_and_do!(self, Token::KeywordIntrinsicType, {
      let pos = self.current_token_position();
      self.advance();
      pos.0
    });

    let (name, end) = match self.current_token {
      Some(Token::Identifier(start, end)) => {
        let name_str = read_string!(self, start, end);

        self.advance();

        (
          IdentifierNode {
            pos: (start, end),
            name: name_str,
          },
          end,
        )
      }
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingTypeNameInTypeDefinition,
        })
      }
    };

    Some(IntrinsicTypeDefNode {
      pos: (start, end),
      visibility: self.current_visibility,
      name,
      generic_type_constraints: Vec::new(),
    })
  }

  fn parse_let_statement(&mut self) -> Option<LetNode> {
    let start = expect_token_and_do!(self, Token::KeywordLet, {
      let (start, _) = self.current_token_position();
      self.advance();
      start
    });

    let pattern = match self.parse_pattern() {
      Some(node) => node,
      _ => todo!(),
    };

    expect_token_and_do!(self, Token::Equal, {
      self.advance();
    });

    let (end, value) = match self.parse_expression() {
      Some(node) => (node.pos.1, node),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingRightHandSideOfAssignment,
        })
      }
    };

    Some(LetNode {
      pos: (start, end),
      pattern,
      value,
    })
  }

  fn parse_list_or_dict(&mut self) -> Option<ExprNode> {
    let start = expect_token_and_do!(self, Token::LeftBracket, {
      let pos = self.current_token_position();
      self.advance();
      pos.0
    });

    let mut list_elements = Vec::new();
    let mut dict_entries = Vec::new();

    while let Some(expr) = self.parse_expression() {
      if current_token_is!(self, Token::Colon) {
        if !list_elements.is_empty() {
          self.error::<ExprNode>(ParseError {
            pos: self.current_token_position(),
            kind: ParseErrorKind::UnexpectedDictValueInArray,
          });
        }

        self.advance();

        match self.parse_expression() {
          Some(val) => dict_entries.push((expr, val)),
          _ => {
            return self.error(ParseError {
              pos: self.current_token_position(),
              kind: ParseErrorKind::MissingDictValue,
            })
          }
        }
      } else {
        if !dict_entries.is_empty() {
          self.error::<ExprNode>(ParseError {
            pos: self.current_token_position(),
            kind: ParseErrorKind::MissingDictValue,
          });
        }

        list_elements.push(expr);
      }

      if current_token_is!(self, Token::Comma) {
        self.advance()
      } else {
        break;
      }
    }

    let kind = if list_elements.is_empty() && dict_entries.is_empty() {
      if current_token_is!(self, Token::Colon) {
        self.advance();
        ExprKind::Dict { entries: vec![] }
      } else {
        ExprKind::List { elements: vec![] }
      }
    } else if list_elements.len() > 0 {
      ExprKind::List {
        elements: list_elements,
      }
    } else {
      ExprKind::Dict {
        entries: dict_entries,
      }
    };

    let end = expect_token_and_do!(self, Token::RightBracket, {
      let pos = self.current_token_position();
      self.advance();
      pos.1
    });

    Some(ExprNode {
      pos: (start, end),
      kind,
      typ: ValueType::Unknown,
    })
  }

  fn parse_match(&mut self) -> Option<ExprNode> {
    let start = match self.current_token {
      Some(Token::KeywordMatch(start, _)) => {
        self.advance();
        start
      }
      _ => unreachable!(),
    };

    let subject = match self.parse_expression() {
      Some(node) => node,
      _ => todo!(),
    };

    self.skip_line_breaks();

    let mut cases = Vec::new();
    let mut match_end = start;

    while let Some(Token::KeywordWhen(case_start, _)) = self.current_token {
      self.advance();

      let pattern = match self.parse_pattern() {
        Some(node) => node,
        _ => todo!(),
      };

      match self.current_token {
        Some(Token::DoubleArrow(..)) => self.advance(),
        _ => todo!(),
      };

      self.skip_line_breaks();

      let (case_end, body) = match self.parse_expression() {
        Some(node) => (node.pos.1, node),
        other => return other,
      };

      self.skip_line_breaks();
      match_end = case_end;

      cases.push(MatchCaseNode {
        pos: (case_start, case_end),
        pattern,
        body,
      });
    }

    if cases.is_empty() {
      self.error::<ExprNode>(ParseError {
        pos: (start, match_end),
        kind: ParseErrorKind::MissingMatchCases,
      });
    }

    Some(ExprNode {
      pos: (start, match_end),
      kind: ExprKind::Match {
        match_: MatchNode {
          pos: (start, match_end),
          subject: Box::new(subject),
          cases,
        },
      },
      typ: ValueType::Unknown,
    })
  }

  fn parse_numeric_literal(&self, start: usize, end: usize, radix: i32) -> i32 {
    let mut result: i32 = 0;
    let mut i: i32 = 1;

    for byte in self.source[start..end].iter().rev() {
      let byte_value = match byte {
        b'0'..=b'9' => byte - 48,
        b'A'..=b'F' => byte - 65,
        b'a'..=b'f' => byte - 97,
        _ => unreachable!(),
      };

      result += (byte_value as i32) * i;
      i *= radix;
    }

    result
  }

  fn parse_octal_number(&mut self) -> Option<LiteralNode> {
    let (start, end, value) = expect_token_and_do!(self, Token::OctalDigits, {
      let (start, end) = self.current_token_position();
      (start, end, self.parse_numeric_literal(start + 2, end, 8))
    });

    self.advance();

    Some(LiteralNode {
      kind: LiteralKind::IntOctal(value),
      pos: (start, end),
    })
  }

  fn parse_operator(&mut self) -> Option<OperatorNode> {
    let (start, end, kind) = match self.current_token {
      Some(Token::Plus(start, end)) => (start, end, OperatorKind::Add),
      Some(Token::Minus(start, end)) => (start, end, OperatorKind::Subtract),

      Some(Token::Star(start, end)) => (start, end, OperatorKind::Multiply),
      Some(Token::ForwardSlash(start, end)) => (start, end, OperatorKind::Divide),
      Some(Token::Percent(start, end)) => (start, end, OperatorKind::Mod),
      Some(Token::DoubleStar(start, end)) => (start, end, OperatorKind::Exponent),

      Some(Token::And(start, end)) => (start, end, OperatorKind::BitwiseAnd),
      Some(Token::Pipe(start, end)) => (start, end, OperatorKind::BitwiseOr),
      Some(Token::Caret(start, end)) => (start, end, OperatorKind::BitwiseXor),
      Some(Token::DoubleLeftAngle(start, end)) => (start, end, OperatorKind::BitwiseLeftShift),
      Some(Token::DoubleRightAngle(start, end)) => (start, end, OperatorKind::BitwiseRightShift),

      Some(Token::DoubleAnd(start, end)) => (start, end, OperatorKind::LogicalAnd),
      Some(Token::DoublePipe(start, end)) => (start, end, OperatorKind::LogicalOr),

      Some(Token::LeftAngle(start, end)) => (start, end, OperatorKind::LessThan),
      Some(Token::RightAngle(start, end)) => (start, end, OperatorKind::GreaterThan),
      Some(Token::LeftAngleEqual(start, end)) => (start, end, OperatorKind::LessThanEquals),
      Some(Token::RightAngleEqual(start, end)) => (start, end, OperatorKind::GreaterThanEquals),
      Some(Token::DoubleEqual(start, end)) => (start, end, OperatorKind::Equals),
      Some(Token::BangEqual(start, end)) => (start, end, OperatorKind::NotEquals),

      Some(Token::DoublePlus(start, end)) => (start, end, OperatorKind::Concat),

      _ => return None,
    };

    self.advance();

    Some(OperatorNode {
      pos: (start, end),
      kind,
    })
  }

  fn parse_pattern(&mut self) -> Option<PatternNode> {
    match self.current_token {
      Some(Token::KeywordMut(start, _)) => {
        self.advance();

        expect_token_and_do!(self, Token::Identifier, {});

        let id_node = self.parse_identifier(false).unwrap();

        Some(PatternNode {
          pos: (start, id_node.pos.1),
          kind: PatternKind::Identifier(id_node, true),
        })
      }

      Some(Token::Identifier(..)) => {
        let id_node = self.parse_identifier(false).unwrap();

        if let Some(arg_pattern) = self.parse_pattern() {
          return Some(PatternNode {
            pos: (id_node.pos.0, arg_pattern.pos.1),
            kind: PatternKind::Constructor(id_node, Box::new(arg_pattern)),
          });
        }

        Some(PatternNode {
          pos: id_node.pos,
          kind: PatternKind::Identifier(id_node, false),
        })
      }

      Some(Token::LeftParen(start, _)) => {
        self.advance();

        let next_token = self.tokenizer.peek();

        match (self.current_token, next_token) {
          (Some(Token::Identifier(..)), Some(Token::Colon(..))) => {
            // It's a labeled tuple!
            let mut labeled_entries = Vec::new();

            while let Some(label) = self.parse_identifier(false) {
              expect_token_and_do!(self, Token::Colon, { self.advance() });

              let pattern = match self.parse_pattern() {
                Some(pattern) => pattern,
                _ => break,
              };

              labeled_entries.push((label, pattern));

              match self.current_token {
                Some(Token::Comma(..)) => self.advance(),
                _ => break,
              }
            }

            let end = expect_token_and_do!(self, Token::RightParen, {
              let (_, end) = self.current_token_position();
              self.advance();
              end
            });

            return Some(PatternNode {
              pos: (start, end),
              kind: PatternKind::LabeledTuple(labeled_entries),
            });
          }
          _ => {}
        }

        let mut entries = Vec::new();

        while let Some(pattern) = self.parse_pattern() {
          entries.push(pattern);

          match self.current_token {
            Some(Token::Comma(..)) => self.advance(),
            _ => break,
          }
        }

        let end = expect_token_and_do!(self, Token::RightParen, {
          let (_, end) = self.current_token_position();
          self.advance();
          end
        });

        Some(PatternNode {
          pos: (start, end),
          kind: PatternKind::UnlabeledTuple(entries),
        })
      }

      Some(Token::Underscore(start, end)) => {
        self.advance();

        Some(PatternNode {
          pos: (start, end),
          kind: PatternKind::Underscore,
        })
      }

      Some(Token::StringLiteral(..)) => self.parse_string().map(|expr_node| match expr_node.kind {
        ExprKind::Literal { literal } => PatternNode {
          pos: literal.pos,
          kind: PatternKind::Literal(literal),
        },
        ExprKind::Interpolation { parts } => PatternNode {
          pos: expr_node.pos,
          kind: PatternKind::Interpolation(parts),
        },
        _ => unreachable!(),
      }),

      Some(Token::DecimalDigits(..)) => self.parse_decimal_number().map(|lit_node| PatternNode {
        pos: lit_node.pos,
        kind: PatternKind::Literal(lit_node),
      }),

      Some(Token::HexDigits(..)) => self.parse_hex_number().map(|lit_node| PatternNode {
        pos: lit_node.pos,
        kind: PatternKind::Literal(lit_node),
      }),

      Some(Token::OctalDigits(..)) => self.parse_octal_number().map(|lit_node| PatternNode {
        pos: lit_node.pos,
        kind: PatternKind::Literal(lit_node),
      }),

      Some(Token::BinaryDigits(..)) => self.parse_binary_number().map(|lit_node| PatternNode {
        pos: lit_node.pos,
        kind: PatternKind::Literal(lit_node),
      }),

      _ => None,
    }
  }

  fn parse_parenthetical(&mut self) -> Option<ExprNode> {
    // "parentheticals" are a little tricky, because they could be a number of things:
    //  - "()" is an empty tuple
    //  - "(expr)" is an expression in parentheses (a grouping),
    //  - "(expr, expr, ...)" is an unlabeled tuple
    //  - "(ident: expr, ...)" is a labeled tuple

    let paren_start = expect_token_and_do!(self, Token::LeftParen, {
      let (start, _) = self.current_token_position();
      self.advance();
      start
    });

    let mut first_expr = None;
    let mut other_exprs = Vec::new();
    let mut labeled = false;
    let mut labeled_entries = Vec::new();

    self.skip_line_breaks();

    while let Some(node) = self.parse_expression() {
      if labeled {
        match node.kind {
          ExprKind::Identifier { ident: label } => {
            expect_token_and_do!(self, Token::Colon, {
              self.advance();
            });

            self.skip_line_breaks();

            if let Some(value) = self.parse_expression() {
              labeled_entries.push((label, value));
            } else {
              self.error::<ExprNode>(ParseError {
                pos: node.pos,
                kind: ParseErrorKind::MissingExpressionAfterLabelInTuple,
              });
            }
          }
          _ => {
            self.error::<ExprNode>(ParseError {
              pos: node.pos,
              kind: ParseErrorKind::MissingLabelInTuple,
            });
          }
        }
      } else if first_expr.is_none() {
        if current_token_is!(self, Token::Colon) {
          self.advance();
          labeled = true;

          match node.kind {
            ExprKind::Identifier { ident: label } => {
              self.skip_line_breaks();

              if let Some(value) = self.parse_expression() {
                labeled_entries.push((label, value));
              } else {
                self.error::<ExprNode>(ParseError {
                  pos: node.pos,
                  kind: ParseErrorKind::MissingExpressionAfterLabelInTuple,
                });
              }
            }
            _ => {
              self.error::<ExprNode>(ParseError {
                pos: node.pos,
                kind: ParseErrorKind::MissingLabelInTuple,
              });
            }
          }
        } else {
          first_expr = Some(node)
        }
      } else {
        other_exprs.push(node);
      }

      self.skip_line_breaks();

      match self.current_token {
        Some(Token::Comma(..)) => {
          self.advance();
          self.skip_line_breaks();
        }
        _ => break,
      }
    }

    self.skip_line_breaks();

    let paren_end = match self.current_token {
      Some(Token::RightParen(_, end)) => end,
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::UnclosedParentheses,
        })
      }
    };

    self.advance();

    if !labeled_entries.is_empty() {
      // If we collected labeled entries, this is a labeled tuple
      return Some(ExprNode {
        pos: (paren_start, paren_end),
        kind: ExprKind::LabeledTuple {
          entries: labeled_entries,
        },
        typ: ValueType::Unknown,
      });
    }

    if first_expr.is_none() {
      // If no expressions were found between the ()s, it's an empty tuple
      return Some(ExprNode {
        pos: (paren_start, paren_end),
        kind: ExprKind::EmptyTuple,
        typ: ValueType::Unknown,
      });
    }

    if other_exprs.is_empty() {
      return Some(ExprNode {
        pos: (paren_start, paren_end),
        kind: ExprKind::Grouping {
          inner: Box::new(first_expr.unwrap()),
        },
        typ: ValueType::Unknown,
      });
    }

    other_exprs.insert(0, first_expr.unwrap());

    Some(ExprNode {
      pos: (paren_start, paren_end),
      kind: ExprKind::UnlabeledTuple {
        entries: other_exprs,
      },
      typ: ValueType::Unknown,
    })
  }

  fn parse_private(&mut self) -> Option<TopLevelStatementNode> {
    let pos = expect_token_and_do!(self, Token::KeywordPrivate, {
      let pos = self.current_token_position();
      self.advance();
      pos
    });

    self.current_visibility = ExportVisibility::Private;

    Some(TopLevelStatementNode {
      pos: pos,
      kind: TopLevelStatementKind::VisibilityMarker(ExportVisibility::Private),
    })
  }

  fn parse_qualified(&mut self) -> Option<ExprNode> {
    let qualifier = self.parse_qualifier().unwrap();

    let ident = expect_token_and_do!(self, Token::Identifier, {
      self.parse_identifier(false).unwrap()
    });

    Some(ExprNode {
      pos: qualifier.pos,
      kind: ExprKind::QualifiedIdentifier {
        qualifier,
        ident: Box::new(ident),
      },
      typ: ValueType::Unknown,
    })
  }

  fn parse_qualifier(&mut self) -> Option<QualifierNode> {
    let (start, end) = expect_token_and_do!(self, Token::Qualifier, {
      let (start, end) = self.current_token_position();
      self.advance();
      (start, end)
    });

    let name = read_string!(self, start, end);

    Some(QualifierNode {
      pos: (start, end),
      name,
    })
  }

  fn parse_regular_expression(&mut self) -> Option<ExprNode> {
    let start = expect_token_and_do!(self, Token::ForwardSlash, {
      let (start, _) = self.current_token_position();
      self.advance();
      start
    });

    self.skip_line_breaks();

    let maybe_reg_expr_node = self.parse_regular_expression_body();

    self.skip_line_breaks();

    let end = expect_token_and_do!(self, Token::ForwardSlash, {
      let (_, end) = self.current_token_position();
      self.advance();
      end
    });

    let regex = match maybe_reg_expr_node {
      Some(expr) => expr,
      None => {
        return self.error(ParseError {
          pos: (start, end),
          kind: ParseErrorKind::EmptyRegularExpression,
        })
      }
    };

    Some(ExprNode {
      pos: (start, end),
      kind: ExprKind::RegExpr { regex },
      typ: ValueType::Unknown,
    })
  }

  fn parse_regular_expression_body(&mut self) -> Option<RegExprNode> {
    let mut first_term = None;
    let mut other_terms = Vec::new();

    let mut term = self.parse_regular_expression_term();

    loop {
      if term.is_some() {
        self.skip_line_breaks();

        if first_term.is_none() {
          first_term = term;
        } else {
          other_terms.push(term.unwrap());
        }

        match self.current_token {
          Some(Token::Pipe(..)) => {
            self.advance();
            term = self.parse_regular_expression_term();
            continue;
          }
          _ => {}
        }
      }

      break;
    }

    if first_term.is_none() {
      return None;
    }

    if other_terms.is_empty() {
      return first_term;
    }

    other_terms.insert(0, first_term.unwrap());
    let start = other_terms.first().unwrap().pos.0;
    let end = other_terms.last().unwrap().pos.1;

    Some(RegExprNode {
      pos: (start, end),
      kind: RegExprKind::Alternation(other_terms),
    })
  }

  fn parse_regular_expression_term(&mut self) -> Option<RegExprNode> {
    let mut first_part = None;
    let mut other_parts = Vec::new();

    loop {
      self.skip_line_breaks();

      let part = match self.current_token {
        Some(Token::Identifier(start, end)) => {
          self.advance();

          let name = read_string!(self, start, end);

          RegExprNode {
            pos: (start, end),
            kind: RegExprKind::CharacterClass(name),
          }
        }

        Some(Token::StringLiteral(start, end)) => {
          self.advance();

          let value = read_string_with_escapes!(self, start, end);

          RegExprNode {
            pos: (start, end),
            kind: RegExprKind::Literal(value),
          }
        }

        Some(Token::LeftParen(start, end)) => {
          self.advance();

          let expr = match self.parse_regular_expression_body() {
            Some(expr) => expr,
            None => {
              return self.error(ParseError {
                pos: (start, end),
                kind: ParseErrorKind::EmptyRegularExpressionGroup,
              })
            }
          };

          expect_token_and_do!(self, Token::RightParen, { self.advance() });

          RegExprNode {
            pos: (start, end),
            kind: RegExprKind::Grouping(Box::new(expr)),
          }
        }

        Some(Token::LeftAngle(start, end)) => {
          self.advance();

          let name = expect_token_and_do!(self, Token::Identifier, {
            let (start, end) = self.current_token_position();
            let name = read_string!(self, start, end);
            self.advance();
            name
          });

          expect_token_and_do!(self, Token::Colon, { self.advance() });

          let expr = match self.parse_regular_expression_body() {
            Some(expr) => expr,
            None => {
              return self.error(ParseError {
                pos: (start, end),
                kind: ParseErrorKind::EmptyRegularExpressionGroup,
              })
            }
          };

          expect_token_and_do!(self, Token::RightAngle, { self.advance() });

          RegExprNode {
            pos: (start, end),
            kind: RegExprKind::NamedCapture(name, Box::new(expr)),
          }
        }

        _ => break,
      };

      let modified_part = match self.current_token {
        Some(Token::Star(_, end)) => {
          self.advance();

          RegExprNode {
            pos: (part.pos.0, end),
            kind: RegExprKind::ZeroOrMore(Box::new(part)),
          }
        }

        Some(Token::Plus(_, end)) => {
          self.advance();

          RegExprNode {
            pos: (part.pos.0, end),
            kind: RegExprKind::OneOrMore(Box::new(part)),
          }
        }

        Some(Token::Question(_, end)) => {
          self.advance();

          RegExprNode {
            pos: (part.pos.0, end),
            kind: RegExprKind::OneOrZero(Box::new(part)),
          }
        }

        Some(Token::LeftBrace(_, _)) => {
          self.advance();

          let mut min_count = None;
          let mut max_count = None;
          let mut has_comma = false;

          if current_token_is!(self, Token::DecimalDigits) {
            let (start, end) = self.current_token_position();
            let value = self.parse_numeric_literal(start, end, 10) as usize;
            min_count = Some(value);
            self.advance();
          }

          if current_token_is!(self, Token::Comma) {
            has_comma = true;

            self.advance();

            if current_token_is!(self, Token::DecimalDigits) {
              let (start, end) = self.current_token_position();
              let value = self.parse_numeric_literal(start, end, 10) as usize;
              max_count = Some(value);
              self.advance();
            }
          }

          let end = expect_token_and_do!(self, Token::RightBrace, {
            let (_, end) = self.current_token_position();
            self.advance();
            end
          });

          match (min_count, max_count, has_comma) {
            (Some(min), None, true) => RegExprNode {
              pos: (part.pos.0, end),
              kind: RegExprKind::AtLeastCount(Box::new(part), min),
            },

            (None, Some(max), true) => RegExprNode {
              pos: (part.pos.0, end),
              kind: RegExprKind::AtMostCount(Box::new(part), max),
            },

            (Some(min), None, false) => RegExprNode {
              pos: (part.pos.0, end),
              kind: RegExprKind::ExactCount(Box::new(part), min),
            },

            (Some(min), Some(max), true) => {
              if min > max {
                self.error::<RegExprNode>(ParseError {
                  pos: (part.pos.0, end),
                  kind: ParseErrorKind::InvalidRegularExpressionCountModifier,
                });
              }

              RegExprNode {
                pos: (part.pos.0, end),
                kind: RegExprKind::RangeCount(Box::new(part), min, max),
              }
            }

            _ => {
              return self.error(ParseError {
                pos: (part.pos.0, end),
                kind: ParseErrorKind::EmptyRegularExpressionCount,
              })
            }
          }
        }

        _ => part,
      };

      self.skip_line_breaks();

      if first_part.is_none() {
        first_part = Some(modified_part);
      } else {
        other_parts.push(modified_part);
      }
    }

    if other_parts.is_empty() {
      if first_part.is_some() {
        return first_part;
      }

      return None;
    }

    match first_part {
      Some(part) => other_parts.insert(0, part),
      None => return None,
    };

    Some(RegExprNode {
      pos: (0, 0),
      kind: RegExprKind::Sequence(other_parts),
    })
  }

  fn parse_statement(&mut self) -> Option<StatementNode> {
    match self.current_token {
      Some(Token::KeywordLet(..)) => self.parse_let_statement().map(|let_node| StatementNode {
        pos: let_node.pos,
        kind: StatementKind::Let(let_node),
      }),
      _ => self.parse_expression().map(|expr_node| StatementNode {
        pos: expr_node.pos,
        kind: StatementKind::Expr(expr_node),
      }),
    }
  }

  fn parse_string(&mut self) -> Option<ExprNode> {
    let (start, end) = expect_token_and_do!(self, Token::StringLiteral, {
      let pos = self.current_token_position();
      self.advance();
      pos
    });

    let value = read_string_with_escapes!(self, start, end);

    let literal = LiteralNode {
      pos: (start, end),
      kind: LiteralKind::Str(value),
    };

    let expr_node = ExprNode {
      pos: (start, end),
      kind: ExprKind::Literal { literal },
      typ: ValueType::Unknown,
    };

    if current_token_is!(self, Token::InterpolationStart) {
      let mut parts = vec![expr_node];
      let mut interpolation_end = end;

      while current_token_is!(self, Token::InterpolationStart) {
        self.advance();

        match self.parse_expression() {
          Some(node) => parts.push(node),
          _ => break,
        }

        expect_token_and_do!(self, Token::InterpolationEnd, {
          self.advance();
        });

        expect_token_and_do!(self, Token::StringLiteral, {
          let (start, end) = self.current_token_position();

          interpolation_end = end;

          let value = read_string_with_escapes!(self, start, end);

          parts.push(ExprNode {
            pos: (start, end),
            kind: ExprKind::Literal {
              literal: LiteralNode {
                pos: (start, end),
                kind: LiteralKind::Str(value),
              },
            },
            typ: ValueType::Unknown,
          });

          self.advance()
        })
      }

      return Some(ExprNode {
        pos: (start, interpolation_end),
        kind: ExprKind::Interpolation { parts },
        typ: ValueType::Unknown,
      });
    }

    Some(expr_node)
  }

  fn parse_struct(&mut self) -> Option<TypeDefNode> {
    let start = expect_token_and_do!(self, Token::KeywordStruct, {
      let pos = self.current_token_position();
      self.advance();
      pos.0
    });

    let name = match self.parse_type_identifier() {
      Some(type_id) => type_id,
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingTypeNameInTypeDefinition,
        })
      }
    };

    self.skip_line_breaks();

    let generic_type_constraints = self.parse_generic_type_constraints().unwrap_or_default();

    let inner = match self.parse_type_expression() {
      Some(type_expr) => type_expr,
      _ => {
        // Assume that the failure to parse the type expression has
        // already generated an error
        return None;
      }
    };

    Some(TypeDefNode {
      pos: (start, inner.pos.1),
      visibility: self.current_visibility,
      kind: TypeDefKind::Struct { inner },
      name,
      generic_type_constraints,
    })
  }

  fn parse_term(&mut self) -> Option<ExprNode> {
    match self.current_token {
      Some(Token::LeftParen(..)) => self.parse_parenthetical(),
      Some(Token::ForwardSlash(..)) => self.parse_regular_expression(),
      Some(Token::Bang(..)) | Some(Token::Minus(..)) | Some(Token::Tilde(..)) => {
        self.parse_unary_operation()
      }
      Some(Token::LeftBrace(..)) => self.parse_block().map(|block| ExprNode {
        pos: block.pos,
        kind: ExprKind::Block { block },
        typ: ValueType::Unknown,
      }),
      Some(Token::LeftBracket(..)) => self.parse_list_or_dict(),
      Some(Token::StringLiteral(..)) => self.parse_string(),
      Some(Token::KeywordMatch(..)) => self.parse_match(),
      Some(Token::Qualifier(..)) => self.parse_qualified(),
      Some(Token::Identifier(..))
      | Some(Token::IdentifierSpecialParam(..))
      | Some(Token::IdentifierSpecialOther(..)) => {
        self.parse_identifier(true).map(|ident| ExprNode {
          pos: ident.pos,
          kind: ExprKind::Identifier { ident },
          typ: ValueType::Unknown,
        })
      }
      Some(Token::DecimalDigits(..)) => self.parse_decimal_number().map(|literal| ExprNode {
        pos: literal.pos,
        kind: ExprKind::Literal { literal },
        typ: ValueType::Unknown,
      }),
      Some(Token::HexDigits(..)) => self.parse_hex_number().map(|literal| ExprNode {
        pos: literal.pos,
        kind: ExprKind::Literal { literal },
        typ: ValueType::Unknown,
      }),
      Some(Token::OctalDigits(..)) => self.parse_octal_number().map(|literal| ExprNode {
        pos: literal.pos,
        kind: ExprKind::Literal { literal },
        typ: ValueType::Unknown,
      }),
      Some(Token::BinaryDigits(..)) => self.parse_binary_number().map(|literal| ExprNode {
        pos: literal.pos,
        kind: ExprKind::Literal { literal },
        typ: ValueType::Unknown,
      }),
      _ => None,
    }
  }

  fn parse_top_level_statement(&mut self) -> Option<TopLevelStatementNode> {
    match self.current_token {
      Some(Token::KeywordLet(..)) => {
        self
          .parse_let_statement()
          .map(|let_node| TopLevelStatementNode {
            pos: let_node.pos,
            kind: TopLevelStatementKind::Let(let_node),
          })
      }
      Some(Token::KeywordDef(..)) => {
        self
          .parse_definition()
          .map(|def_node| TopLevelStatementNode {
            pos: def_node.pos,
            kind: TopLevelStatementKind::Def(def_node),
          })
      }
      Some(Token::KeywordIntrinsicDef(..)) => {
        self
          .parse_intrinsic_definition()
          .map(|intrinsic_def_node| TopLevelStatementNode {
            pos: intrinsic_def_node.pos,
            kind: TopLevelStatementKind::IntrinsicDef(intrinsic_def_node),
          })
      }
      Some(Token::KeywordAlias(..)) => {
        self
          .parse_alias()
          .map(|type_def_node| TopLevelStatementNode {
            pos: type_def_node.pos,
            kind: TopLevelStatementKind::TypeDef(type_def_node),
          })
      }
      Some(Token::KeywordEnum(..)) => {
        self
          .parse_enum()
          .map(|type_def_node| TopLevelStatementNode {
            pos: type_def_node.pos,
            kind: TopLevelStatementKind::TypeDef(type_def_node),
          })
      }
      Some(Token::KeywordIntrinsicType(..)) => {
        self
          .parse_intrinsic_type()
          .map(|intrinsic_type_def_node| TopLevelStatementNode {
            pos: intrinsic_type_def_node.pos,
            kind: TopLevelStatementKind::IntrinsicTypeDef(intrinsic_type_def_node),
          })
      }
      Some(Token::KeywordStruct(..)) => {
        self
          .parse_struct()
          .map(|type_def_node| TopLevelStatementNode {
            pos: type_def_node.pos,
            kind: TopLevelStatementKind::TypeDef(type_def_node),
          })
      }
      Some(Token::KeywordTrait(..)) => {
        self
          .parse_trait()
          .map(|type_def_node| TopLevelStatementNode {
            pos: type_def_node.pos,
            kind: TopLevelStatementKind::TypeDef(type_def_node),
          })
      }
      Some(Token::KeywordPrivate(..)) => self.parse_private(),
      Some(Token::KeywordInternal(..)) => self.parse_internal(),
      _ => self
        .parse_expression()
        .map(|expr_node| TopLevelStatementNode {
          pos: expr_node.pos,
          kind: TopLevelStatementKind::Expr(expr_node),
        }),
    }
  }

  fn parse_trait(&mut self) -> Option<TypeDefNode> {
    let start = expect_token_and_do!(self, Token::KeywordTrait, {
      let pos = self.current_token_position();
      self.advance();
      pos.0
    });

    let name = match self.parse_type_identifier() {
      Some(type_id) => type_id,
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingTypeNameInTypeDefinition,
        })
      }
    };

    self.skip_line_breaks();

    let generic_type_constraints = self.parse_generic_type_constraints().unwrap_or_default();

    self.skip_line_breaks();

    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut end = start;

    'outer: while let Some(Token::Dot(..)) = self.current_token {
      self.advance();

      let mut signature = Vec::new();

      while current_token_is!(self, Token::Identifier) {
        let part_name = self.parse_identifier(false).unwrap();

        if current_token_is!(self, Token::Colon) {
          self.advance();

          // If there's a :: here, it's only valid if there has only been a single part
          // so far (since it must be a field, not a method).
          if signature.is_empty() {
            match self.parse_type_expression() {
              Some(field_type) => {
                end = field_type.pos.1;
                fields.push((part_name, field_type));
              }
              _ => {
                return self.error(ParseError {
                  pos: self.current_token_position(),
                  kind: ParseErrorKind::IncompleteMethodSignature,
                })
              }
            }
          } else {
            self.error::<TypeDefNode>(ParseError {
              pos: self.current_token_position(),
              kind: ParseErrorKind::UnexpectedToken(Token::Dot(0, 0)),
            });
          }

          self.skip_line_breaks();

          continue 'outer;
        }

        match self.parse_type_expression() {
          Some(part_param) => signature.push((part_name, Box::new(part_param))),
          _ => {
            return self.error(ParseError {
              pos: self.current_token_position(),
              kind: ParseErrorKind::IncompleteMethodSignature,
            })
          }
        }
      }

      expect_token_and_do!(self, Token::Arrow, {
        self.advance();
      });

      match self.parse_type_expression() {
        Some(return_type) => {
          end = return_type.pos.1;
          methods.push((signature, return_type));
        }
        _ => {
          return self.error(ParseError {
            pos: self.current_token_position(),
            kind: ParseErrorKind::IncompleteMethodSignature,
          })
        }
      }

      self.skip_line_breaks();
    }

    self.skip_line_breaks();

    Some(TypeDefNode {
      pos: (start, end),
      visibility: self.current_visibility,
      kind: TypeDefKind::Trait { fields, methods },
      name,
      generic_type_constraints,
    })
  }

  fn parse_type_func(&mut self) -> Option<TypeExprNode> {
    let start = expect_token_and_do!(self, Token::LeftBrace, {
      let (start, _) = self.current_token_position();
      self.advance();
      start
    });

    self.skip_line_breaks();

    let param_type = match self.parse_type_expression() {
      Some(type_expr) => Box::new(type_expr),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingTypeInBlockType,
        })
      }
    };

    expect_token_and_do!(self, Token::Arrow, {
      self.advance();
    });

    let return_type = match self.parse_type_expression() {
      Some(type_expr) => Box::new(type_expr),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingReturnType,
        })
      }
    };

    let end = expect_token_and_do!(self, Token::RightBrace, {
      let (_, end) = self.current_token_position();
      self.advance();
      end
    });

    Some(TypeExprNode {
      pos: (start, end),
      kind: TypeExprKind::Func(param_type, return_type),
      typ: ValueType::Unknown,
    })
  }

  fn parse_type_expression(&mut self) -> Option<TypeExprNode> {
    match self.current_token {
      Some(Token::Identifier(..)) => self.parse_type_identifier().map(|type_id| TypeExprNode {
        pos: type_id.pos,
        kind: TypeExprKind::Single(type_id),
        typ: ValueType::Unknown,
      }),
      Some(Token::LeftParen(..)) => self.parse_type_parenthetical(),
      Some(Token::LeftBrace(..)) => self.parse_type_func(),
      _ => None,
    }
  }

  fn parse_type_identifier(&mut self) -> Option<TypeIdentifierNode> {
    let (start, mut end) = expect_token_and_do!(self, Token::Identifier, {
      let pos = self.current_token_position();
      self.advance();
      pos
    });

    let name = read_string!(self, start, end);

    let mut generics = Vec::new();

    if current_token_is!(self, Token::LeftAngle) {
      self.advance();

      while let Some(type_node) = self.parse_type_expression() {
        generics.push(type_node);

        match self.current_token {
          Some(Token::Comma(..)) => self.advance(),
          _ => break,
        }
      }

      end = expect_token_and_do!(self, Token::RightAngle, {
        let pos = self.current_token_position();
        self.advance();
        pos.1
      });
    }

    Some(TypeIdentifierNode {
      pos: (start, end),
      name,
      generics,
      constraints: None,
    })
  }

  fn parse_type_parenthetical(&mut self) -> Option<TypeExprNode> {
    let start = expect_token_and_do!(self, Token::LeftParen, {
      let pos = self.current_token_position();
      self.advance();
      pos.0
    });

    self.skip_line_breaks();

    let mut first_entry = None;
    let mut other_entries = Vec::new();
    let mut labeled = false;
    let mut labeled_entries = Vec::new();

    while let Some(type_node) = self.parse_type_expression() {
      if labeled {
        match type_node.kind {
          TypeExprKind::Single(label) => {
            let label_ident = IdentifierNode {
              name: label.name,
              pos: label.pos,
            };

            expect_token_and_do!(self, Token::Colon, { self.advance() });

            if let Some(value) = self.parse_type_expression() {
              labeled_entries.push((label_ident, value));
            } else {
              self.error::<TypeExprNode>(ParseError {
                pos: type_node.pos,
                kind: ParseErrorKind::MissingExpressionAfterLabelInTuple,
              });
            }
          }

          _ => {
            self.error::<TypeExprNode>(ParseError {
              pos: type_node.pos,
              kind: ParseErrorKind::MissingLabelInTuple,
            });
          }
        }
      } else if first_entry.is_none() {
        if current_token_is!(self, Token::Colon) {
          self.advance();
          labeled = true;

          match type_node.kind {
            TypeExprKind::Single(label) => {
              let label_ident = IdentifierNode {
                name: label.name,
                pos: label.pos,
              };

              if let Some(value) = self.parse_type_expression() {
                labeled_entries.push((label_ident, value));
              } else {
                self.error::<TypeExprNode>(ParseError {
                  pos: type_node.pos,
                  kind: ParseErrorKind::MissingExpressionAfterLabelInTuple,
                });
              }
            }

            _ => {
              self.error::<TypeExprNode>(ParseError {
                pos: type_node.pos,
                kind: ParseErrorKind::MissingLabelInTuple,
              });
            }
          }
        } else {
          first_entry = Some(type_node)
        }
      } else {
        other_entries.push(type_node);
      }

      match self.current_token {
        Some(Token::Comma(..)) => self.advance(),
        _ => break,
      }

      self.skip_line_breaks();
    }

    self.skip_line_breaks();

    let end = expect_token_and_do!(self, Token::RightParen, {
      let pos = self.current_token_position();
      self.advance();
      pos.1
    });

    if labeled {
      return Some(TypeExprNode {
        pos: (start, end),
        kind: TypeExprKind::LabeledTuple(labeled_entries),
        typ: ValueType::Unknown,
      });
    }

    if first_entry.is_none() {
      return Some(TypeExprNode {
        pos: (start, end),
        kind: TypeExprKind::EmptyTuple,
        typ: ValueType::Unknown,
      });
    }

    if other_entries.is_empty() {
      return Some(TypeExprNode {
        pos: (start, end),
        kind: TypeExprKind::Grouping(Box::new(first_entry.unwrap())),
        typ: ValueType::Unknown,
      });
    }

    other_entries.insert(0, first_entry.unwrap());

    Some(TypeExprNode {
      pos: (start, end),
      kind: TypeExprKind::UnlabeledTuple(other_entries),
      typ: ValueType::Unknown,
    })
  }

  fn parse_unary_operation(&mut self) -> Option<ExprNode> {
    let op_node = match self.parse_operator() {
      Some(op_node) => op_node,
      None => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::UnexpectedTokenExpectedOperator,
        })
      }
    };

    // TODO only allow correct precedence here
    let expr_node = match self.parse_expression() {
      Some(node) => Box::new(node),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingExpressionAfterOperator,
        })
      }
    };

    Some(ExprNode {
      pos: (op_node.pos.0, expr_node.pos.1),
      kind: ExprKind::UnaryOperation {
        op: Box::new(op_node),
        right: expr_node,
      },
      typ: ValueType::Unknown,
    })
  }

  fn parse_use_statement(&mut self) -> Option<UseNode> {
    let start = expect_token_and_do!(self, Token::KeywordUse, {
      let (start, _) = self.current_token_position();
      self.advance();
      start
    });

    let qualifier = if current_token_is!(self, Token::Qualifier) {
      self.parse_qualifier()
    } else {
      None
    };

    let (module_name, end) = expect_token_and_do!(self, Token::ImportPath, {
      let (start, end) = self.current_token_position();
      let name_str = read_string!(self, start, end);
      self.advance();
      (name_str, end)
    });

    Some(UseNode {
      pos: (start, end),
      module_name,
      qualifier,
    })
  }
}
