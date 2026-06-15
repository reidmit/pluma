use atty::Stream;
use std::env;

// Whether diagnostic output should carry ANSI color. Off when stdout isn't a
// TTY or when `NO_COLOR=1` is set. The actual styling lives in the compiler's
// `Palette`; this just decides which palette the CLI hands the renderer.
pub fn should_colorize() -> bool {
	if !atty::is(Stream::Stdout) {
		return false;
	}

	match env::var("NO_COLOR") {
		Ok(value) => value != "1",
		_ => true,
	}
}

// The terminal's column count, used to window over-wide source lines in diagnostics
// so their caret rows stay aligned. Diagnostics print to stderr, so the size is read
// from that handle. `None` (output isn't a terminal, e.g. piped to a file) leaves
// lines untrimmed — full lines are friendlier to tooling and `grep`.
pub fn terminal_width() -> Option<usize> {
	terminal_size::terminal_size_of(std::io::stderr()).map(|(terminal_size::Width(w), _)| w as usize)
}
