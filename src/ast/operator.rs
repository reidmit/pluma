use crate::tokens::Token;

pub enum Operator {
  Addition,
  Subtraction,
  Multiplication,
  Division,
  Exponentiation,
}

impl Operator {
  pub fn infix_binding_power(&self) -> (u8, u8) {
    use Operator::*;

    // if left < right, it's left-associative
    // if left > right, it's right-associative
    match &self {
      Addition | Subtraction => (1, 2),
      Multiplication | Division => (3, 4),
      Exponentiation => (6, 5),
    }
  }
}

impl TryFrom<Token> for Operator {
  type Error = ();

  fn try_from(token: Token) -> Result<Operator, Self::Error> {
    match token {
      Token::Plus(..) => Ok(Operator::Addition),
      Token::Minus(..) => Ok(Operator::Subtraction),
      Token::Star(..) => Ok(Operator::Multiplication),
      Token::ForwardSlash(..) => Ok(Operator::Division),
      Token::DoubleStar(..) => Ok(Operator::Exponentiation),
      _ => Err(()),
    }
  }
}

impl std::fmt::Debug for Operator {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match &self {
      Operator::Addition => write!(f, "op-add"),
      Operator::Subtraction => write!(f, "op-subtract"),
      Operator::Multiplication => write!(f, "op-multiply"),
      Operator::Division => write!(f, "op-divide"),
      Operator::Exponentiation => write!(f, "op-exponent"),
    }
  }
}
