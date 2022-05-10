use crate::tokens::Token;

pub enum Operator {
  Addition,
  Subtraction,
  Multiplication,
  Division,
  Remainder,
  Exponentiation,
  LogicalAnd,
  LogicalOr,
  LessThan,
  GreaterThan,
  LessThanEquals,
  GreaterThanEquals,
  Equality,
  Inequality,
}

impl Operator {
  pub fn from_token(token: Token) -> Option<Operator> {
    match token {
      Token::Plus(..) => Some(Operator::Addition),
      Token::Minus(..) => Some(Operator::Subtraction),
      Token::Star(..) => Some(Operator::Multiplication),
      Token::ForwardSlash(..) => Some(Operator::Division),
      Token::Percent(..) => Some(Operator::Remainder),
      Token::DoubleStar(..) => Some(Operator::Exponentiation),
      Token::DoubleAnd(..) => Some(Operator::LogicalAnd),
      Token::DoublePipe(..) => Some(Operator::LogicalOr),
      Token::LeftAngle(..) => Some(Operator::LessThan),
      Token::RightAngle(..) => Some(Operator::GreaterThan),
      Token::LeftAngleEqual(..) => Some(Operator::LessThanEquals),
      Token::RightAngleEqual(..) => Some(Operator::GreaterThanEquals),
      Token::DoubleEqual(..) => Some(Operator::Equality),
      Token::BangEqual(..) => Some(Operator::Inequality),
      _ => None,
    }
  }

  pub fn infix_binding_power(&self) -> (u8, u8) {
    use Operator::*;

    // if left < right, it's left-associative
    // if left > right, it's right-associative
    match &self {
      LogicalOr => (20, 21),
      LogicalAnd => (30, 31),
      Equality | Inequality => (40, 41),
      LessThan | LessThanEquals | GreaterThan | GreaterThanEquals => (50, 51),
      Addition | Subtraction => (60, 61),
      Multiplication | Division | Remainder => (70, 71),
      Exponentiation => (81, 80),
    }
  }
}

impl std::fmt::Debug for Operator {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    use Operator::*;

    match &self {
      Addition => write!(f, "op-add"),
      Subtraction => write!(f, "op-subtract"),
      Multiplication => write!(f, "op-multiply"),
      Division => write!(f, "op-divide"),
      Remainder => write!(f, "op-remainder"),
      Exponentiation => write!(f, "op-exponent"),
      LogicalAnd => write!(f, "op-logical-and"),
      LogicalOr => write!(f, "op-logical-or"),
      LessThan => write!(f, "op-less-than"),
      GreaterThan => write!(f, "op-greater-than"),
      LessThanEquals => write!(f, "op-less-than-equals"),
      GreaterThanEquals => write!(f, "op-greater-than-equals"),
      Equality => write!(f, "op-equality"),
      Inequality => write!(f, "op-inequality"),
    }
  }
}
