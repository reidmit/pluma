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

/// ANSI styling for the CLI's own chrome (the `dev` dashboard, the `build` summary),
/// gated on a single `on` flag so the same call sites work colored or plain. Mirrors
/// the compiler's `Palette` but for output that isn't a diagnostic.
#[derive(Clone, Copy)]
pub struct Style {
	pub on: bool,
}

impl Style {
	/// Style for the current terminal — colored when stdout is an ANSI-capable TTY.
	pub fn detect() -> Self {
		Style {
			on: should_colorize(),
		}
	}

	fn paint(self, codes: &str, text: &str) -> String {
		if self.on {
			format!("\x1b[{codes}m{text}\x1b[0m")
		} else {
			text.to_string()
		}
	}
	pub fn bold(self, t: &str) -> String {
		self.paint("1", t)
	}
	pub fn dim(self, t: &str) -> String {
		self.paint("2", t)
	}
	pub fn green(self, t: &str) -> String {
		self.paint("1;32", t)
	}
	pub fn red(self, t: &str) -> String {
		self.paint("1;31", t)
	}
	pub fn yellow(self, t: &str) -> String {
		self.paint("33", t)
	}
	pub fn cyan(self, t: &str) -> String {
		self.paint("36", t)
	}
}

// The terminal's column count, used to window over-wide source lines in diagnostics
// so their caret rows stay aligned. Diagnostics print to stderr, so the size is read
// from that handle. `None` (output isn't a terminal, e.g. piped to a file) leaves
// lines untrimmed — full lines are friendlier to tooling and `grep`.
pub fn terminal_width() -> Option<usize> {
	terminal_size::terminal_size_of(std::io::stderr()).map(|(terminal_size::Width(w), _)| w as usize)
}
