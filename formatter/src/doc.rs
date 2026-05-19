// A minimal Wadler/Hughes pretty-printer.
//
// Builder-side concepts:
//   Text(s)              raw text; never broken
//   HardLine             unconditional newline + indent
//   Line                 " " in flat mode, newline + indent in break mode
//   SoftBreak            "" in flat mode, newline + indent in break mode
//   IfFlat(a, b)         emit `a` in flat mode, `b` in break mode
//   Nest(n, d)           increase indent by `n` levels for `d`
//   Concat(parts)        sequence
//   Group(d)             try to lay `d` out flat; fall back to break if it
//                        doesn't fit or contains a HardLine
//
// Layout uses two passes (the usual stack-based formulation): when we hit a
// Group, simulate the remainder of the page in flat mode and check whether
// everything up to the next mandatory line break fits within the width
// budget. If yes, lay the group out flat; otherwise lay it out in break
// mode.

#[derive(Clone)]
pub enum Doc {
	Nil,
	Text(String),
	HardLine,
	Line,
	SoftBreak,
	IfFlat(Box<Doc>, Box<Doc>),
	Nest(usize, Box<Doc>),
	Concat(Vec<Doc>),
	Group(Box<Doc>),
}

// Tabs are one byte but render at some user-configurable width. We use this
// constant only when measuring "does this group fit on a line" — it doesn't
// affect the bytes emitted (those are literal tabs).
const TAB_WIDTH_FOR_FIT: usize = 4;

pub fn nil() -> Doc {
	Doc::Nil
}

pub fn text(s: impl Into<String>) -> Doc {
	Doc::Text(s.into())
}

pub fn hardline() -> Doc {
	Doc::HardLine
}

pub fn line() -> Doc {
	Doc::Line
}

pub fn softbreak() -> Doc {
	Doc::SoftBreak
}

pub fn if_flat(flat: Doc, broken: Doc) -> Doc {
	Doc::IfFlat(Box::new(flat), Box::new(broken))
}

pub fn nest(d: Doc) -> Doc {
	Doc::Nest(1, Box::new(d))
}

pub fn concat(parts: Vec<Doc>) -> Doc {
	Doc::Concat(parts)
}

pub fn group(d: Doc) -> Doc {
	Doc::Group(Box::new(d))
}

pub fn join(sep: Doc, items: Vec<Doc>) -> Doc {
	let mut out = Vec::with_capacity(items.len() * 2);
	for (i, item) in items.into_iter().enumerate() {
		if i > 0 {
			out.push(sep.clone());
		}
		out.push(item);
	}
	concat(out)
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
	Flat,
	Break,
}

#[derive(Clone)]
struct Item<'a> {
	indent: usize,
	mode: Mode,
	doc: &'a Doc,
}

pub fn render(doc: &Doc, width: usize) -> String {
	let mut out = String::new();
	let mut col: usize = 0;
	let mut stack: Vec<Item> = vec![Item {
		indent: 0,
		mode: Mode::Break,
		doc,
	}];

	while let Some(item) = stack.pop() {
		match item.doc {
			Doc::Nil => {}
			Doc::Text(s) => {
				out.push_str(s);
				col += s.chars().count();
			}
			Doc::HardLine => {
				emit_newline(&mut out, item.indent);
				col = item.indent * TAB_WIDTH_FOR_FIT;
			}
			Doc::Line => match item.mode {
				Mode::Flat => {
					out.push(' ');
					col += 1;
				}
				Mode::Break => {
					emit_newline(&mut out, item.indent);
					col = item.indent * TAB_WIDTH_FOR_FIT;
				}
			},
			Doc::SoftBreak => match item.mode {
				Mode::Flat => {}
				Mode::Break => {
					emit_newline(&mut out, item.indent);
					col = item.indent * TAB_WIDTH_FOR_FIT;
				}
			},
			Doc::IfFlat(flat, broken) => match item.mode {
				Mode::Flat => stack.push(Item {
					indent: item.indent,
					mode: item.mode,
					doc: flat,
				}),
				Mode::Break => stack.push(Item {
					indent: item.indent,
					mode: item.mode,
					doc: broken,
				}),
			},
			Doc::Nest(n, inner) => stack.push(Item {
				indent: item.indent + n,
				mode: item.mode,
				doc: inner,
			}),
			Doc::Concat(parts) => {
				for p in parts.iter().rev() {
					stack.push(Item {
						indent: item.indent,
						mode: item.mode,
						doc: p,
					});
				}
			}
			Doc::Group(inner) => {
				// Try flat. Build a probe of the same shape as the real stack
				// after this group's contents would be pushed in flat mode, and
				// check whether everything up to the next mandatory newline
				// fits in the remaining budget.
				let flat_item = Item {
					indent: item.indent,
					mode: Mode::Flat,
					doc: inner,
				};
				let mut probe: Vec<Item> = stack.clone();
				probe.push(flat_item.clone());
				let remaining = (width as i64) - (col as i64);
				if fits(remaining, probe) {
					stack.push(flat_item);
				} else {
					stack.push(Item {
						indent: item.indent,
						mode: Mode::Break,
						doc: inner,
					});
				}
			}
		}
	}

	out
}

fn fits(mut budget: i64, mut stack: Vec<Item>) -> bool {
	while budget >= 0 {
		let Some(item) = stack.pop() else {
			return true;
		};
		match item.doc {
			Doc::Nil => {}
			Doc::Text(s) => {
				budget -= s.chars().count() as i64;
			}
			Doc::HardLine => {
				return match item.mode {
					Mode::Flat => false,
					Mode::Break => true,
				};
			}
			Doc::Line => match item.mode {
				Mode::Flat => {
					budget -= 1;
				}
				Mode::Break => return true,
			},
			Doc::SoftBreak => match item.mode {
				Mode::Flat => {}
				Mode::Break => return true,
			},
			Doc::IfFlat(flat, broken) => {
				let doc = match item.mode {
					Mode::Flat => flat.as_ref(),
					Mode::Break => broken.as_ref(),
				};
				stack.push(Item {
					indent: item.indent,
					mode: item.mode,
					doc,
				});
			}
			Doc::Nest(n, inner) => stack.push(Item {
				indent: item.indent + n,
				mode: item.mode,
				doc: inner,
			}),
			Doc::Concat(parts) => {
				for p in parts.iter().rev() {
					stack.push(Item {
						indent: item.indent,
						mode: item.mode,
						doc: p,
					});
				}
			}
			Doc::Group(inner) => stack.push(Item {
				indent: item.indent,
				mode: item.mode,
				doc: inner,
			}),
		}
	}
	false
}

fn emit_newline(out: &mut String, indent: usize) {
	out.push('\n');
	for _ in 0..indent {
		out.push('\t');
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn text_emits_verbatim() {
		assert_eq!(render(&text("hello"), 80), "hello");
	}

	#[test]
	fn group_picks_flat_when_it_fits() {
		let d = group(concat(vec![text("a"), line(), text("b")]));
		assert_eq!(render(&d, 80), "a b");
	}

	#[test]
	fn group_breaks_when_too_wide() {
		let d = group(concat(vec![
			text("aaaa"),
			line(),
			text("bbbb"),
			line(),
			text("cccc"),
		]));
		assert_eq!(render(&d, 8), "aaaa\nbbbb\ncccc");
	}

	#[test]
	fn nest_indents_with_tabs() {
		let d = group(concat(vec![
			text("{"),
			nest(concat(vec![line(), text("body")])),
			line(),
			text("}"),
		]));
		assert_eq!(render(&d, 4), "{\n\tbody\n}");
	}

	#[test]
	fn hardline_forces_break() {
		let d = group(concat(vec![text("a"), hardline(), text("b")]));
		assert_eq!(render(&d, 80), "a\nb");
	}

	#[test]
	fn if_flat_swaps_per_mode() {
		let mk = || group(concat(vec![text("a"), if_flat(text(", "), line()), text("b")]));
		assert_eq!(render(&mk(), 80), "a, b");
		assert_eq!(render(&mk(), 2), "a\nb");
	}
}
