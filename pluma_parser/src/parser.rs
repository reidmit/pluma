use crate::parse_error::*;
use crate::tokenizer::Tokenizer;
use crate::tokens::Token;
use pluma_ast::*;

macro_rules! current_token_is {
  ($self:ident, $tokType:path) => {
    match $self.current_token {
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
          pos: tok.get_location(),
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
  def_body_stack: i8,
  current_token: Option<Token>,
  prev_token: Option<Token>,
}

impl<'a> Parser<'a> {
  pub fn new(source: &'a Vec<u8>, tokenizer: Tokenizer<'a>) -> Parser<'a> {
    return Parser {
      source,
      tokenizer,
      index: 0,
      errors: Vec::new(),
      def_body_stack: 0,
      current_token: None,
      prev_token: None,
    };
  }

  pub fn parse_module(&mut self) -> (ModuleNode, Vec<UseNode>, Vec<ParseError>) {
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

    let start = body.first().map_or(0, |node| node.pos.0);
    let end = body.last().map_or(0, |node| node.pos.1);

    let module_node = ModuleNode {
      pos: (start, end),
      body,
    };

    (module_node, imports, self.errors.clone())
  }

  fn advance(&mut self) {
    self.prev_token = self.current_token;
    self.current_token = self.tokenizer.next();
    self.index += 1;
  }

  fn skip_line_breaks(&mut self) {
    while current_token_is!(self, Token::LineBreak) {
      self.advance()
    }
  }

  fn current_token_position(&self) -> (usize, usize) {
    match self.current_token {
      Some(token) => token.get_location(),
      _ => match self.prev_token {
        Some(token) => token.get_location(),
        _ => (0, 0),
      },
    }
  }

  fn enter_def_body(&mut self) {
    self.def_body_stack += 1;
  }

  fn exit_def_body(&mut self) {
    self.def_body_stack -= 1;
  }

  fn in_def_body(&mut self) -> bool {
    self.def_body_stack > 0
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
    })
  }

  fn parse_binary_number(&mut self) -> Option<LiteralNode> {
    let (start, end, value) = expect_token_and_do!(self, Token::BinaryDigits, {
      let (start, end) = self.current_token_position();
      (start, end, self.parse_numeric_literal(start, end, 2))
    });

    self.advance();

    Some(LiteralNode {
      kind: LiteralKind::IntBinary(value),
      pos: (start, end),
    })
  }

  fn parse_block(&mut self) -> Option<ExprNode> {
    let block_start = expect_token_and_do!(self, Token::LeftBrace, {
      let (start, _) = self.current_token_position();
      self.advance();
      start
    });

    self.skip_line_breaks();

    let mut params = Vec::new();
    let mut body = Vec::new();

    match self.parse_statement() {
      Some(stmt) => {
        if current_token_is!(self, Token::Comma) || current_token_is!(self, Token::DoubleArrow) {
          match &stmt.kind {
            StatementKind::Expr(expr) => match &expr.kind {
              ExprKind::Identifier(ident) => {
                if current_token_is!(self, Token::Comma) {
                  self.advance();
                }

                params.push(IdentifierNode {
                  pos: ident.pos,
                  name: ident.name.clone(),
                });
              }
              _ => body.push(stmt),
            },
            _ => body.push(stmt),
          }
        } else {
          body.push(stmt);
        }
      }
      None => {}
    }

    if !params.is_empty() {
      while let Some(ident) = self.parse_identifier() {
        params.push(ident);

        match self.current_token {
          Some(Token::Comma(..)) => self.advance(),
          Some(Token::DoubleArrow(..)) => {
            break;
          }
          _ => break,
        }
      }

      expect_token_and_do!(self, Token::DoubleArrow, {
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

    Some(ExprNode {
      pos: (block_start, block_end),
      kind: ExprKind::Block { params, body },
      typ: ValueType::Unknown,
    })
  }

  fn parse_binary_operation(&mut self, last_term: ExprNode) -> Option<ExprNode> {
    let op_node = match self.current_token {
      Some(Token::Operator(start, end))
      | Some(Token::LeftAngle(start, end))
      | Some(Token::RightAngle(start, end)) => {
        let name = read_string!(self, start, end);
        self.advance();

        Box::new(OperatorNode {
          pos: (start, end),
          name,
        })
      }
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::UnexpectedToken(Token::Operator(0, 0)),
        })
      }
    };

    self.skip_line_breaks();

    let (end, next_term) = match self.parse_operator_branch() {
      Some(term) => (term.pos.1, Box::new(term)),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingExpressionAfterOperator,
        })
      }
    };

    Some(ExprNode {
      pos: (last_term.pos.0, end),
      kind: ExprKind::BinaryOperation {
        left: Box::new(last_term),
        op: op_node,
        right: next_term,
      },
      typ: ValueType::Unknown,
    })
  }

  fn parse_call(&mut self, last_expr: ExprNode) -> Option<CallNode> {
    // At this point, last_expr is either an Identifier (e.g. print in `print "hello"`)
    // or a Chain (e.g. `a 1 . b` in `a 1 . b 2.`).

    let start = last_expr.pos.0;
    let mut args = Vec::new();

    // Grab the first argument (the next expression).
    match self.parse_term() {
      Some(arg) => args.push(arg),
      _ => return None,
    }

    let callee = match last_expr.kind {
      ExprKind::Identifier(first_callee_part) => {
        let start = first_callee_part.pos.0;

        let mut rest_callee_parts = Vec::new();

        // If the last expr was an identifier, we allow multi-part names here

        // If there is an identifier now, it means this is a call to a multi-part name.
        while current_token_is!(self, Token::Identifier) {
          match self.parse_identifier() {
            Some(next_callee_part) => {
              rest_callee_parts.push(next_callee_part);

              // Grab the argument for this part
              match self.parse_term() {
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

        let (pos, kind) = if rest_callee_parts.len() > 0 {
          let mut all_parts = vec![first_callee_part];
          all_parts.append(&mut rest_callee_parts);

          (
            (start, all_parts.last().unwrap().pos.1),
            ExprKind::MultiPartIdentifier(all_parts),
          )
        } else {
          (
            first_callee_part.pos,
            ExprKind::Identifier(first_callee_part),
          )
        };

        ExprNode {
          pos,
          kind,
          typ: ValueType::Unknown,
        }
      }

      ExprKind::FieldAccess { receiver, field } => {
        let mut rest_callee_parts = Vec::new();

        // If there is an identifier now, it means this is a call to a multi-part name.
        while current_token_is!(self, Token::Identifier) {
          match self.parse_identifier() {
            Some(next_callee_part) => {
              rest_callee_parts.push(next_callee_part);

              // Grab the argument for this part
              match self.parse_term() {
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

        let mut method_parts = vec![field];

        if rest_callee_parts.len() > 0 {
          method_parts.append(&mut rest_callee_parts);
        }

        ExprNode {
          pos: last_expr.pos,
          kind: ExprKind::MethodAccess {
            receiver,
            method_parts,
          },
          typ: ValueType::Unknown,
        }
      }
      _ => last_expr,
    };

    Some(CallNode {
      pos: (start, args.last().unwrap().pos.1),
      callee: Box::new(callee),
      args,
      typ: ValueType::Unknown,
    })
  }

  fn parse_chain(&mut self, last_expr: ExprNode) -> Option<ExprNode> {
    expect_token_and_do!(self, Token::Dot, { self.advance() });

    match self.parse_term() {
      Some(ExprNode { kind, pos, .. }) => match kind {
        ExprKind::Identifier(ident) => {
          return Some(ExprNode {
            pos: (last_expr.pos.0, pos.1),
            kind: ExprKind::FieldAccess {
              receiver: Box::new(last_expr),
              field: ident,
            },
            typ: ValueType::Unknown,
          })
        }

        // ExprKind::Call(call) => {
        //   return Some(ExprNode {
        //     pos: (last_expr.pos.0, pos.1),
        //     kind: ExprKind::MethodAccess {
        //       receiver: Box::new(last_expr),
        //       field: ident,
        //     },
        //     typ: ValueType::Unknown,
        //   });
        // }
        _ => {
          return self.error(ParseError {
            pos,
            kind: ParseErrorKind::UnexpectedExpressionAfterDot,
          })
        }
      },

      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingExpressionAfterDot,
        })
      }
    };
  }

  fn parse_decimal_number(&mut self) -> Option<LiteralNode> {
    let (start, end) = expect_token_and_do!(self, Token::DecimalDigits, {
      let pos = self.current_token_position();
      self.advance();
      pos
    });

    if current_token_is!(self, Token::Dot) {
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
      Some(token) => token.get_location().1,
      _ => start,
    };

    Some(IntrinsicDefNode {
      pos: (start, end),
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

    self.enter_def_body();

    let (params, body, end) = match self.parse_block() {
      Some(ExprNode {
        kind: ExprKind::Block { params, body },
        pos,
        ..
      }) => (params, body, pos.1),
      _ => return None,
    };

    self.exit_def_body();

    Some(DefNode {
      pos: (start, end),
      kind,
      return_type,
      generic_type_constraints,
      params,
      body,
    })
  }

  fn parse_generic_type_constraints(&mut self) -> Option<GenericTypeConstraints> {
    let mut generic_type_constraints = Vec::new();

    if current_token_is!(self, Token::KeywordWhere) {
      self.advance();

      while let Some(generic_name) = self.parse_identifier() {
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
    let mut binary_operator = None;
    let mut signature: Signature = Vec::new();

    if current_token_is!(self, Token::Dot) {
      // If we have a dot now, we know the first ident was a receiver type
      receiver = Some(type_ident);
      self.advance();
    } else if current_token_is!(self, Token::Operator) {
      // If we have an operator now, this is a binary operator definition
      receiver = Some(type_ident);

      let (start, end) = self.current_token_position();
      let name = read_string!(self, start, end);

      binary_operator = Some(OperatorNode {
        pos: (start, end),
        name,
      });

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

    // Binary operator defs can only have a single type identifier after the operator (and a return value),
    // so handle that here.
    if let Some(op) = binary_operator {
      let left_type = receiver.unwrap();

      let right_type = match self.parse_type_identifier() {
        Some(t) => t,
        None => {
          return self.error(ParseError {
            pos: self.current_token_position(),
            kind: ParseErrorKind::IncompleteMethodSignature,
          })
        }
      };

      return Some(DefKind::BinaryOperator {
        left: Box::new(left_type),
        op: Box::new(op),
        right: Box::new(right_type),
      });
    }

    // Now, collect any remaining parts
    while current_token_is!(self, Token::Identifier) {
      let part_name = self.parse_identifier().unwrap();

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

      match self.parse_identifier() {
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
      kind: TypeDefKind::Enum { variants },
      name,
      generic_type_constraints,
    })
  }

  fn parse_expression(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_operator_branch();

    loop {
      if expr.is_some() {
        match self.current_token {
          Some(Token::Operator(..)) | Some(Token::LeftAngle(..)) | Some(Token::RightAngle(..)) => {
            expr = self.parse_binary_operation(expr.unwrap());
            continue;
          }
          Some(Token::DoubleColon(..)) => {
            expr = self.parse_type_assertion(expr.unwrap());
            continue;
          }
          _ => {}
        }
      }

      break;
    }

    expr
  }

  fn parse_hex_number(&mut self) -> Option<LiteralNode> {
    let (start, end, value) = expect_token_and_do!(self, Token::HexDigits, {
      let (start, end) = self.current_token_position();
      (start, end, self.parse_numeric_literal(start, end, 16))
    });

    self.advance();

    Some(LiteralNode {
      kind: LiteralKind::IntHex(value),
      pos: (start, end),
    })
  }

  fn parse_identifier(&mut self) -> Option<IdentifierNode> {
    let (start, end) = match self.current_token {
      Some(Token::Identifier(start, end)) => {
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

    expect_token_and_do!(self, Token::Equals, {
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
        ExprKind::Dict(vec![])
      } else {
        ExprKind::List(vec![])
      }
    } else if list_elements.len() > 0 {
      ExprKind::List(list_elements)
    } else {
      ExprKind::Dict(dict_entries)
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

    while let Some(Token::Pipe(case_start, _)) = self.current_token {
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
      kind: ExprKind::Match(MatchNode {
        pos: (start, match_end),
        subject: Box::new(subject),
        cases,
      }),
      typ: ValueType::Unknown,
    })
  }

  fn parse_numeric_literal(&self, start: usize, end: usize, radix: i32) -> i32 {
    let mut result: i32 = 0;
    let mut i: i32 = 1;

    for byte in self.source[start..end].iter().rev() {
      let byte_value = match byte {
        b'0'..=b'9' => byte - 48,
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
      (start, end, self.parse_numeric_literal(start, end, 8))
    });

    self.advance();

    Some(LiteralNode {
      kind: LiteralKind::IntOctal(value),
      pos: (start, end),
    })
  }

  fn parse_operator_branch(&mut self) -> Option<ExprNode> {
    let mut expr = self.parse_term();

    loop {
      // Skip line breaks if there are any, but keep track of whether or not
      // there were. Line breaks are not allowed between a callee and its
      // argument, but they are allowed between the receiver and the "." in
      // a chain expression.
      let index_before_breaks = self.index;
      self.skip_line_breaks();
      let skipped_any_line_breaks = self.index != index_before_breaks;

      if expr.is_some() {
        match self.current_token {
          Some(Token::Dot(..)) => {
            expr = self.parse_chain(expr.unwrap());
            continue;
          }
          Some(Token::LeftParen(..))
          | Some(Token::OctalDigits(..))
          | Some(Token::DecimalDigits(..))
          | Some(Token::BinaryDigits(..))
          | Some(Token::HexDigits(..))
          | Some(Token::StringLiteral(..))
          | Some(Token::Identifier(..))
          | Some(Token::LeftBracket(..))
          | Some(Token::LeftBrace(..))
            if !skipped_any_line_breaks =>
          {
            expr = self.parse_call(expr.unwrap()).map(|call_node| ExprNode {
              pos: call_node.pos,
              kind: ExprKind::Call(call_node),
              typ: ValueType::Unknown,
            });
            continue;
          }
          _ => {}
        }
      }

      break;
    }

    expr
  }

  fn parse_pattern(&mut self) -> Option<PatternNode> {
    match self.current_token {
      Some(Token::KeywordMut(start, _)) => {
        self.advance();

        expect_token_and_do!(self, Token::Identifier, {});

        let id_node = self.parse_identifier().unwrap();

        Some(PatternNode {
          pos: (start, id_node.pos.1),
          kind: PatternKind::Identifier(id_node, true),
        })
      }

      Some(Token::Identifier(..)) => {
        let id_node = self.parse_identifier().unwrap();

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
          kind: PatternKind::Tuple(entries),
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
        ExprKind::Literal(lit_node) => PatternNode {
          pos: lit_node.pos,
          kind: PatternKind::Literal(lit_node),
        },
        ExprKind::Interpolation(expr_nodes) => PatternNode {
          pos: expr_node.pos,
          kind: PatternKind::Interpolation(expr_nodes),
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
    let paren_start = expect_token_and_do!(self, Token::LeftParen, {
      let (start, _) = self.current_token_position();
      self.advance();
      start
    });

    let mut first_expr = None;
    let mut other_exprs = Vec::new();

    while let Some(node) = self.parse_expression() {
      if first_expr.is_none() {
        first_expr = Some(node)
      } else {
        other_exprs.push(node);
      }

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

    if first_expr.is_none() {
      return Some(ExprNode {
        pos: (paren_start, paren_end),
        kind: ExprKind::EmptyTuple,
        typ: ValueType::Unknown,
      });
    }

    if other_exprs.is_empty() {
      return Some(ExprNode {
        pos: (paren_start, paren_end),
        kind: ExprKind::Grouping(Box::new(first_expr.unwrap())),
        typ: ValueType::Unknown,
      });
    }

    other_exprs.insert(0, first_expr.unwrap());

    Some(ExprNode {
      pos: (paren_start, paren_end),
      kind: ExprKind::Tuple(other_exprs),
      typ: ValueType::Unknown,
    })
  }

  fn parse_private(&mut self) -> Option<TopLevelStatementNode> {
    let pos = expect_token_and_do!(self, Token::KeywordPrivate, {
      let pos = self.current_token_position();
      self.advance();
      pos
    });

    Some(TopLevelStatementNode {
      pos: pos,
      kind: TopLevelStatementKind::PrivateMarker,
    })
  }

  fn parse_return_statement(&mut self) -> Option<ReturnNode> {
    let start = expect_token_and_do!(self, Token::KeywordReturn, {
      let (start, end) = self.current_token_position();
      self.advance();

      if !self.in_def_body() {
        self.error::<ReturnNode>(ParseError {
          pos: (start, end),
          kind: ParseErrorKind::ReturnOutsideDefinitionBody,
        });
      }

      start
    });

    let expr = match self.parse_expression() {
      Some(node) => node,
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingExpressionAfterReturn,
        })
      }
    };

    Some(ReturnNode {
      pos: (start, expr.pos.1),
      value: expr,
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

    let reg_expr_node = match maybe_reg_expr_node {
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
      kind: ExprKind::RegExpr(reg_expr_node),
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
        Some(Token::Operator(start, end)) => {
          let op = &self.source[start..end];

          match op {
            b"*" => {
              self.advance();

              RegExprNode {
                pos: (part.pos.0, end),
                kind: RegExprKind::ZeroOrMore(Box::new(part)),
              }
            }

            b"+" => {
              self.advance();

              RegExprNode {
                pos: (part.pos.0, end),
                kind: RegExprKind::OneOrMore(Box::new(part)),
              }
            }

            b"?" => {
              self.advance();

              RegExprNode {
                pos: (part.pos.0, end),
                kind: RegExprKind::OneOrZero(Box::new(part)),
              }
            }

            _ => part,
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
      Some(Token::KeywordReturn(..)) => {
        self
          .parse_return_statement()
          .map(|expr_node| StatementNode {
            pos: expr_node.pos,
            kind: StatementKind::Return(expr_node),
          })
      }
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

    let lit_node = LiteralNode {
      pos: (start, end),
      kind: LiteralKind::Str(value),
    };

    let expr_node = ExprNode {
      pos: (start, end),
      kind: ExprKind::Literal(lit_node),
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
            kind: ExprKind::Literal(LiteralNode {
              pos: (start, end),
              kind: LiteralKind::Str(value),
            }),
            typ: ValueType::Unknown,
          });

          self.advance()
        })
      }

      return Some(ExprNode {
        pos: (start, interpolation_end),
        kind: ExprKind::Interpolation(parts),
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

    let mut fields = Vec::new();

    expect_token_and_do!(self, Token::LeftParen, {
      self.advance();
    });

    self.skip_line_breaks();

    while let Some(Token::Identifier(..)) = self.current_token {
      let ident = match self.parse_identifier() {
        Some(node) => node,
        _ => break,
      };

      expect_token_and_do!(self, Token::DoubleColon, {
        self.advance();
      });

      match self.parse_type_expression() {
        Some(expr) => fields.push((ident, expr)),
        _ => {
          // Assume that the failure to parse the type expression has
          // already generated an error
          return None;
        }
      };

      if current_token_is!(self, Token::Comma) {
        self.advance();
      } else {
        break;
      }

      self.skip_line_breaks();
    }

    self.skip_line_breaks();

    let end = expect_token_and_do!(self, Token::RightParen, {
      let pos = self.current_token_position();
      self.advance();
      pos.1
    });

    if fields.is_empty() {
      return self.error(ParseError {
        pos: (start, end),
        kind: ParseErrorKind::MissingStructFields,
      });
    }

    Some(TypeDefNode {
      pos: (start, end),
      kind: TypeDefKind::Struct { fields },
      name,
      generic_type_constraints,
    })
  }

  fn parse_term(&mut self) -> Option<ExprNode> {
    match self.current_token {
      Some(Token::LeftParen(..)) => self.parse_parenthetical(),
      Some(Token::ForwardSlash(..)) => self.parse_regular_expression(),
      Some(Token::Operator(..)) => self.parse_unary_operation(),
      Some(Token::LeftBrace(..)) => self.parse_block(),
      Some(Token::LeftBracket(..)) => self.parse_list_or_dict(),
      Some(Token::StringLiteral(..)) => self.parse_string(),
      Some(Token::KeywordMatch(..)) => self.parse_match(),
      Some(Token::Underscore(..)) => self.parse_underscore(),
      Some(Token::Identifier(..)) => {
        self
          .parse_identifier()
          .map(|id_node| match self.current_token {
            Some(Token::Equals(..)) => {
              self.advance();

              let expr = self.parse_expression().unwrap();

              ExprNode {
                pos: (id_node.pos.0, expr.pos.1),
                kind: ExprKind::Assignment {
                  left: Box::new(id_node),
                  right: Box::new(expr),
                },
                typ: ValueType::Unknown,
              }
            }
            _ => ExprNode {
              pos: id_node.pos,
              kind: ExprKind::Identifier(id_node),
              typ: ValueType::Unknown,
            },
          })
      }
      Some(Token::DecimalDigits(..)) => self.parse_decimal_number().map(|lit_node| ExprNode {
        pos: lit_node.pos,
        kind: ExprKind::Literal(lit_node),
        typ: ValueType::Unknown,
      }),
      Some(Token::HexDigits(..)) => self.parse_hex_number().map(|lit_node| ExprNode {
        pos: lit_node.pos,
        kind: ExprKind::Literal(lit_node),
        typ: ValueType::Unknown,
      }),
      Some(Token::OctalDigits(..)) => self.parse_octal_number().map(|lit_node| ExprNode {
        pos: lit_node.pos,
        kind: ExprKind::Literal(lit_node),
        typ: ValueType::Unknown,
      }),
      Some(Token::BinaryDigits(..)) => self.parse_binary_number().map(|lit_node| ExprNode {
        pos: lit_node.pos,
        kind: ExprKind::Literal(lit_node),
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
        let part_name = self.parse_identifier().unwrap();

        if current_token_is!(self, Token::DoubleColon) {
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

    if fields.is_empty() && methods.is_empty() {
      return self.error(ParseError {
        pos: self.current_token_position(),
        kind: ParseErrorKind::MissingTraitConstraints,
      });
    }

    Some(TypeDefNode {
      pos: (start, end),
      kind: TypeDefKind::Trait { fields, methods },
      name,
      generic_type_constraints,
    })
  }

  fn parse_type_assertion(&mut self, last_term: ExprNode) -> Option<ExprNode> {
    expect_token_and_do!(self, Token::DoubleColon, {
      self.advance();
    });

    self.skip_line_breaks();

    let (end, asserted_type) = match self.parse_type_expression() {
      Some(type_expr) => (type_expr.pos.1, type_expr),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingTypeInTypeAssertion,
        })
      }
    };

    Some(ExprNode {
      pos: (last_term.pos.0, end),
      kind: ExprKind::TypeAssertion {
        expr: Box::new(last_term),
        asserted_type,
      },
      typ: ValueType::Unknown,
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
          kind: ParseErrorKind::MissingType,
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
    })
  }

  fn parse_type_parenthetical(&mut self) -> Option<TypeExprNode> {
    let start = expect_token_and_do!(self, Token::LeftParen, {
      let pos = self.current_token_position();
      self.advance();
      pos.0
    });

    let mut first_entry = None;
    let mut other_entries = Vec::new();

    while let Some(type_node) = self.parse_type_expression() {
      if first_entry.is_none() {
        first_entry = Some(type_node)
      } else {
        other_entries.push(type_node);
      }

      match self.current_token {
        Some(Token::Comma(..)) => self.advance(),
        _ => break,
      }
    }

    let end = expect_token_and_do!(self, Token::RightParen, {
      let pos = self.current_token_position();
      self.advance();
      pos.1
    });

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
      kind: TypeExprKind::Tuple(other_entries),
      typ: ValueType::Unknown,
    })
  }

  fn parse_unary_operation(&mut self) -> Option<ExprNode> {
    let op_node = expect_token_and_do!(self, Token::Operator, {
      let pos = self.current_token_position();
      let name = read_string!(self, pos.0, pos.1);
      self.advance();

      Box::new(OperatorNode { pos, name })
    });

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
        op: op_node,
        right: expr_node,
      },
      typ: ValueType::Unknown,
    })
  }

  fn parse_underscore(&mut self) -> Option<ExprNode> {
    let pos = expect_token_and_do!(self, Token::Underscore, {
      let pos = self.current_token_position();
      self.advance();
      pos
    });

    Some(ExprNode {
      pos,
      kind: ExprKind::Underscore,
      typ: ValueType::Unknown,
    })
  }

  fn parse_use_statement(&mut self) -> Option<UseNode> {
    let start = expect_token_and_do!(self, Token::KeywordUse, {
      let (start, _) = self.current_token_position();
      self.advance();
      start
    });

    let module_name = expect_token_and_do!(self, Token::ImportPath, {
      let (start, end) = self.current_token_position();
      let name_str = read_string!(self, start, end);
      self.advance();
      name_str
    });

    expect_token_and_do!(self, Token::KeywordAs, {
      self.advance();
    });

    let qualifier = match self.parse_identifier() {
      Some(node) => Box::new(node),
      _ => {
        return self.error(ParseError {
          pos: self.current_token_position(),
          kind: ParseErrorKind::MissingQualifierAfterAs,
        })
      }
    };

    Some(UseNode {
      pos: (start, qualifier.pos.1),
      module_name,
      qualifier,
    })
  }
}
