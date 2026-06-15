// The single source of truth for rendering a `Diagnostic` to text. Both the CLI
// (`cli/src/printing.rs`, with color) and the `tests/errors` snapshot suite
// (plain) go through here, so the test corpus guards exactly what users see.
//
// One box-drawing rail runs down the left margin. It opens with a bare `│` under
// the header; any help/notes tee off the top (`├─`); the source excerpt hangs
// below (a gutter `│`, with `┆` marking skipped lines); and it closes with an
// arrowhead pointing at the source location (`╰─𜱶`):
//
//   error[E0100]: Name `lenght` is not defined.
//      │
//      ├─𜱶 help: did you mean `length`?
//      │
//    3 │ def main = lenght
//      │            ^^^^^^
//      │
//      ╰─𜱶 tests/errors/name-typo/main.pa:3:12

use crate::diagnostic::{Diagnostic, Label};
use crate::location::Range;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// Controls colorization. The CLI builds an `ansi()` palette (after its own
// atty/NO_COLOR check) and the test suite builds a `plain()` one, so the two
// outputs are identical modulo escape codes.
pub struct Palette {
	color: bool,
}

impl Palette {
	pub fn plain() -> Self {
		Palette { color: false }
	}

	pub fn ansi() -> Self {
		Palette { color: true }
	}

	fn paint(&self, codes: &str, text: &str) -> String {
		if self.color {
			format!("\x1b[{}m{}\x1b[0m", codes, text)
		} else {
			text.to_string()
		}
	}

	fn bold_red(&self, text: &str) -> String {
		self.paint("1;31", text)
	}

	fn bold_yellow(&self, text: &str) -> String {
		self.paint("1;33", text)
	}

	fn bold_blue(&self, text: &str) -> String {
		self.paint("1;34", text)
	}

	fn bold_cyan(&self, text: &str) -> String {
		self.paint("1;36", text)
	}

	fn dim(&self, text: &str) -> String {
		self.paint("2", text)
	}

	// Interprets the backtick-delimited code spans embedded in message strings
	// (`` `option` ``, ``keyword `def` ``). With color, each matched pair becomes
	// its inner text painted as a code span and the ticks are dropped — color is
	// the emphasis, so the delimiters that stood in for it are redundant. Without
	// color, the text is returned untouched so the ticks remain the emphasis (and
	// the `tests/errors` snapshots stay readable). Only balanced pairs transform;
	// a lone backtick is left literal.
	fn code_spans(&self, text: &str) -> String {
		if !self.color {
			return text.to_string();
		}
		let mut out = String::with_capacity(text.len());
		let mut rest = text;
		while let Some(open) = rest.find('`') {
			match rest[open + 1..].find('`') {
				Some(rel_close) => {
					let close = open + 1 + rel_close;
					out.push_str(&rest[..open]);
					out.push_str(&self.code(&rest[open + 1..close]));
					rest = &rest[close + 1..];
				}
				// Unbalanced trailing backtick: emit the remainder verbatim.
				None => break,
			}
		}
		out.push_str(rest);
		out
	}

	// Emphasis for inline code/type fragments. Cyan keeps code spans distinct from
	// both the red error carets and the blue help/note labels.
	fn code(&self, text: &str) -> String {
		self.bold_cyan(text)
	}
}

// Renders all diagnostics into one string (blank line between each). `load`
// maps a source path to its contents; results are cached so a file shared by
// several diagnostics is read once. A `None` from `load` (synthetic path, e.g.
// stdin) drops the source excerpt — the header, help, and notes still render.
//
// `max_width`, when set, caps how wide a source line may be drawn (in columns):
// a longer line is windowed around its caret with `…` on the clipped ends, so the
// caret row stays aligned instead of being thrown off by the terminal wrapping
// the line. `None` draws every line in full (piped output, snapshot tests).
pub fn render_diagnostics(
	diagnostics: &[Diagnostic],
	mut load: impl FnMut(&Path) -> Option<String>,
	palette: &Palette,
	max_width: Option<usize>,
) -> String {
	let cwd = std::env::current_dir().unwrap_or_default();
	let mut cache: HashMap<PathBuf, Option<Vec<String>>> = HashMap::new();
	let mut out = String::new();

	for (i, diagnostic) in diagnostics.iter().enumerate() {
		if i > 0 {
			out.push('\n');
		}
		render_one(
			diagnostic, &mut load, palette, &cwd, max_width, &mut cache, &mut out,
		);
	}

	out
}

fn render_one(
	diagnostic: &Diagnostic,
	load: &mut impl FnMut(&Path) -> Option<String>,
	palette: &Palette,
	cwd: &Path,
	max_width: Option<usize>,
	cache: &mut HashMap<PathBuf, Option<Vec<String>>>,
	out: &mut String,
) {
	use std::fmt::Write;

	let is_error = diagnostic.is_error();
	let severity = if is_error { "error" } else { "warning" };
	let paint_severity = |text: &str| {
		if is_error {
			palette.bold_red(text)
		} else {
			palette.bold_yellow(text)
		}
	};

	// Header: `error[E0103]: message` (code omitted when absent).
	let header_label = match diagnostic.code {
		Some(code) => format!("{}[{}]", severity, code),
		None => severity.to_string(),
	};
	let _ = writeln!(
		out,
		"{}: {}",
		paint_severity(&header_label),
		palette.code_spans(&diagnostic.message)
	);

	// Resolve the source lines for the file this diagnostic points at.
	let source = diagnostic.module_path.as_ref().and_then(|path| {
		cache
			.entry(path.clone())
			.or_insert_with(|| load(path).map(|s| s.lines().map(|l| l.to_string()).collect()))
			.clone()
	});

	let Some(range) = diagnostic.range else {
		// No location to anchor a rail on (e.g. an ad-hoc CLI error). Render any
		// help/notes as a simple indented trailer.
		render_railless_trailer(diagnostic, palette, out);
		return;
	};
	let Some(path) = diagnostic.module_path.as_ref() else {
		render_railless_trailer(diagnostic, palette, out);
		return;
	};

	// Width of the line-number column. The rail's `│` sits one space to its
	// right, at column `w + 1`; corners and trailers align to the same column.
	let mut max_line = range.end.line;
	for label in &diagnostic.labels {
		max_line = max_line.max(label.range.end.line);
	}
	let w = (max_line + 1).to_string().len();
	let rail_indent = " ".repeat(w + 1);

	// The location the rail's closing arrowhead points at (1-based).
	let display_path = path.strip_prefix(cwd).unwrap_or(path);
	let location = format!(
		"{}:{}:{}",
		display_path.display(),
		range.start.line + 1,
		range.start.col + 1
	);

	// Help and notes lead the rail, stacked just under the header. Each is a tee
	// (`├─`) — never a corner — since the rail always continues down through the
	// snippet to the closing arrowhead.
	let mut trailers: Vec<(String, &str)> = Vec::new();
	if let Some(help) = &diagnostic.help {
		trailers.push(("help:".to_string(), help.as_str()));
	}
	for note in &diagnostic.notes {
		trailers.push(("note:".to_string(), note.as_str()));
	}

	// Open the rail, then tee off the trailers.
	let _ = writeln!(out, "{}{}", rail_indent, palette.dim("│"));
	for (label, text) in &trailers {
		let _ = writeln!(
			out,
			"{}{} {} {}",
			rail_indent,
			palette.dim("├─𜱶"),
			palette.bold_blue(label),
			palette.code_spans(text)
		);
	}

	// The source excerpt. A `│` spacer precedes it only when trailers sit above.
	// No spacer follows: the snippet ends in a caret row, which already sets the
	// closer apart. With no snippet, a spacer stands in so the closer doesn't butt
	// against the trailers (or the opener).
	if let Some(lines) = &source {
		if !trailers.is_empty() {
			let _ = writeln!(out, "{}{}", rail_indent, palette.dim("│"));
		}
		render_snippet(
			range,
			&diagnostic.labels,
			lines,
			w,
			is_error,
			palette,
			max_width,
			out,
		);
	} else if !trailers.is_empty() {
		let _ = writeln!(out, "{}{}", rail_indent, palette.dim("│"));
	}

	// Close the rail with an arrowhead pointing at the source location.
	let _ = writeln!(
		out,
		"{}{} {}",
		rail_indent,
		palette.dim("╰─𜱶"),
		palette.dim(&location)
	);
}

// Help/notes for a diagnostic with no source location to hang a rail on.
fn render_railless_trailer(diagnostic: &Diagnostic, palette: &Palette, out: &mut String) {
	use std::fmt::Write;
	if let Some(help) = &diagnostic.help {
		let _ = writeln!(
			out,
			"  {} {}",
			palette.bold_blue("help:"),
			palette.code_spans(help)
		);
	}
	for note in &diagnostic.notes {
		let _ = writeln!(
			out,
			"  {} {}",
			palette.bold_blue("note:"),
			palette.code_spans(note)
		);
	}
}

// Emits the source-excerpt body: each referenced line plus its caret row, in
// ascending order, with a dashed rail (`┆`) standing in for skipped lines. The
// caller owns the surrounding rail (opening corner + separators + close).
fn render_snippet(
	primary: Range,
	labels: &[Label],
	lines: &[String],
	w: usize,
	is_error: bool,
	palette: &Palette,
	max_width: Option<usize>,
	out: &mut String,
) {
	use std::fmt::Write;

	let paint_caret = |text: &str| {
		if is_error {
			palette.bold_red(text)
		} else {
			palette.bold_yellow(text)
		}
	};
	let rail_indent = " ".repeat(w + 1);

	// Column budget for the source text itself. A line row is `{line_no} │ {text}`
	// and the caret row `{indent}│ {carets}` — both put the text at column `w + 3`,
	// so that prefix comes off the terminal width. A floor keeps a usable window on
	// very narrow terminals; `None` means "don't clip".
	let avail = match max_width {
		Some(mw) => mw.saturating_sub(w + 3).max(16),
		None => usize::MAX,
	};

	// Collect every (line, start_col, span, caption, is_primary) marker. The
	// primary marker uses the severity color; secondary labels are blue.
	struct Marker {
		line: usize,
		start_col: usize,
		span: usize,
		caption: Option<String>,
		primary: bool,
	}

	let mut markers: Vec<Marker> = vec![Marker {
		line: primary.start.line,
		start_col: primary.start.col,
		span: caret_span(primary, lines),
		caption: None,
		primary: true,
	}];
	for label in labels {
		markers.push(Marker {
			line: label.range.start.line,
			start_col: label.range.start.col,
			span: caret_span(label.range, lines),
			caption: Some(label.message.clone()),
			primary: false,
		});
	}
	markers.sort_by_key(|m| (m.line, m.start_col));

	let mut prev_line: Option<usize> = None;
	for marker in &markers {
		// A dashed rail segment when markers skip non-adjacent source lines.
		if let Some(prev) = prev_line {
			if marker.line > prev + 1 {
				let _ = writeln!(out, "{}{}", rail_indent, palette.dim("┆"));
			}
		}
		prev_line = Some(marker.line);

		let Some(text) = lines.get(marker.line) else {
			continue;
		};
		// Tabs render as single spaces so the caret column matches the byte offset.
		let expanded = text.replace('\t', " ");
		let (shown, caret_start, caret_span) =
			clip_line(&expanded, marker.start_col, marker.span, avail);
		let line_no = format!("{:>w$}", marker.line + 1, w = w);
		let _ = writeln!(
			out,
			"{} {} {}",
			palette.dim(&line_no),
			palette.dim("│"),
			shown
		);

		let pad = " ".repeat(caret_start);
		let carets = "^".repeat(caret_span.max(1));
		let painted = if marker.primary {
			paint_caret(&carets)
		} else {
			palette.bold_blue(&carets)
		};
		let caption = match &marker.caption {
			Some(c) if !c.is_empty() => {
				let c = if marker.primary {
					paint_caret(c)
				} else {
					palette.bold_blue(c)
				};
				format!(" {}", c)
			}
			_ => String::new(),
		};
		let _ = writeln!(
			out,
			"{}{} {}{}{}",
			rail_indent,
			palette.dim("│"),
			pad,
			painted,
			caption
		);
	}
}

// Windows a too-wide source line down to a slice around its caret so the caret row
// beneath stays aligned with the text it points at instead of being thrown off by
// the terminal wrapping the line. `avail` is the column budget for the line text
// (the rail/gutter prefix is already subtracted); a clipped end gets a `…`. Columns
// are counted in `char`s — for ASCII source (the common case) that matches the byte
// offsets the caret uses, and for the rare multibyte line it keeps the caret
// truthful. Returns the (possibly trimmed) text with the caret's start column and
// span relative to it.
fn clip_line(line: &str, start_col: usize, span: usize, avail: usize) -> (String, usize, usize) {
	let n = line.chars().count();
	if n <= avail {
		return (line.to_string(), start_col, span);
	}

	let chars: Vec<char> = line.chars().collect();
	// The caret's start/end as char indices (incoming columns are byte offsets).
	let byte_to_char = |byte: usize| line[..byte.min(line.len())].chars().count();
	let cstart = byte_to_char(start_col);
	let cend = byte_to_char((start_col + span).min(line.len()))
		.max(cstart + 1)
		.min(n);

	// A few columns of lead kept before the caret when the left has to be clipped.
	let margin = 8usize;

	// Show from the start unless the caret would fall past the right edge (a column
	// is reserved there for the `…`); otherwise slide right to bring it into view.
	let mut lo = if cend <= avail.saturating_sub(1) {
		0
	} else {
		cstart.saturating_sub(margin)
	};

	// Fit the window to the budget, reserving a column for each `…` actually drawn.
	let mut hi = (lo + avail.saturating_sub((lo > 0) as usize)).min(n);
	if hi < n {
		hi = (lo + avail.saturating_sub((lo > 0) as usize + 1)).min(n);
	}
	// When the caret sits near the end the window reaches the line end with budget to
	// spare — pull the left edge back to fill it with leading context.
	if hi == n && lo > 0 {
		lo = lo.min(n.saturating_sub(avail.saturating_sub(1)));
	}

	let left = lo > 0;
	let right = hi < n;
	let mut shown = String::new();
	if left {
		shown.push('…');
	}
	shown.extend(&chars[lo..hi]);
	if right {
		shown.push('…');
	}

	// Re-anchor the caret in the trimmed text: a left `…` shifts everything one column
	// right, and a caret clipped off the left edge collapses onto the first shown column.
	let lead = left as usize;
	let new_start = lead + cstart.saturating_sub(lo);
	let last_text_col = lead + hi.saturating_sub(lo); // exclusive, before any right `…`
	let new_span = (lead + cend.saturating_sub(lo))
		.min(last_text_col)
		.saturating_sub(new_start)
		.max(1);
	(shown, new_start, new_span)
}

// Caret width for a range: exact for single-line ranges, else to end-of-line.
fn caret_span(range: Range, lines: &[String]) -> usize {
	if range.start.line == range.end.line {
		range.end.col.saturating_sub(range.start.col).max(1)
	} else {
		lines
			.get(range.start.line)
			.map(|l| l.len().saturating_sub(range.start.col).max(1))
			.unwrap_or(1)
	}
}
