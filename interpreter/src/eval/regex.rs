// Translate a Pluma `RegexNode` AST into a pattern string the `regex` crate
// can compile. Quantifiers use the standard syntax. Groupings emit as
// non-capturing `(?:...)` so they don't interfere with anyone reading named
// captures.

use compiler::ast::{RegexKind, RegexNode};

pub fn compile(node: &RegexNode) -> Result<regex::Regex, String> {
	let pattern = build(node);
	regex::Regex::new(&pattern).map_err(|e| format!("invalid regex: {}", e))
}

fn build(node: &RegexNode) -> String {
	match &node.kind {
		RegexKind::Literal(s) => regex::escape(s),
		RegexKind::CharacterClass(c) => format!("[{}]", c),
		RegexKind::OneOrMore(inner) => format!("(?:{})+", build(inner)),
		RegexKind::ZeroOrMore(inner) => format!("(?:{})*", build(inner)),
		RegexKind::OneOrZero(inner) => format!("(?:{})?", build(inner)),
		RegexKind::ExactCount(inner, n) => format!("(?:{}){{{}}}", build(inner), n),
		RegexKind::AtLeastCount(inner, n) => format!("(?:{}){{{},}}", build(inner), n),
		RegexKind::AtMostCount(inner, n) => format!("(?:{}){{0,{}}}", build(inner), n),
		RegexKind::RangeCount(inner, min, max) => {
			format!("(?:{}){{{},{}}}", build(inner), min, max)
		}
		RegexKind::Grouping(inner) => format!("(?:{})", build(inner)),
		RegexKind::Sequence(parts) => parts.iter().map(build).collect(),
		RegexKind::Alternation(parts) => {
			let joined: Vec<_> = parts.iter().map(build).collect();
			format!("(?:{})", joined.join("|"))
		}
		RegexKind::NamedCapture(name, inner) => format!("(?P<{}>{})", name, build(inner)),
	}
}
