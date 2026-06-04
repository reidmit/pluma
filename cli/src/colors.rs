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
