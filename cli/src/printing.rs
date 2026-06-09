use crate::colors;
use compiler::{Diagnostic, Palette, render_diagnostics};
use std::fs;
use std::path::Path;

pub fn print_error<E: std::fmt::Display>(message: E) {
	print_diagnostics(vec![Diagnostic::error(message)])
}

pub fn print_diagnostics(diagnostics: Vec<Diagnostic>) {
	if diagnostics.is_empty() {
		return;
	}
	eprint!("{}", render_diagnostics_string(&diagnostics));
}

/// Render diagnostics to a (possibly colorized) string, instead of printing them.
/// The `pluma dev` dashboard embeds this in its status panel; `print_diagnostics`
/// is the same thing piped to stderr.
pub fn render_diagnostics_string(diagnostics: &[Diagnostic]) -> String {
	if diagnostics.is_empty() {
		return String::new();
	}

	let palette = if colors::should_colorize() {
		Palette::ansi()
	} else {
		Palette::plain()
	};

	// The renderer reads source straight from disk. A path that doesn't exist
	// (synthetic module, e.g. stdin via the formatter) yields `None`, and the
	// renderer falls back to the message + help/notes without an excerpt.
	render_diagnostics(
		diagnostics,
		|path: &Path| fs::read_to_string(path).ok(),
		&palette,
	)
}
