/// Represents a single position in a source code file. Lines are indexed
/// starting at 0. Col refers to the byte offset within the line.
#[derive(Clone, Copy)]
pub struct Point {
	pub line: usize,
	pub col: usize,
}

impl Point {
	/// Creates a new point at given line and col.
	pub fn at(line: usize, col: usize) -> Self {
		Self { line, col }
	}

	/// Creates a new point at line 0, col 0.
	pub fn zero() -> Self {
		Self { line: 0, col: 0 }
	}
}

/// Represents a span between two points in the same source code file. Start
/// and end may be equal, in which case the range is considered "collapsed".
#[derive(Clone, Copy)]
pub struct Range {
	pub start: Point,
	pub end: Point,
}

impl Range {
	/// Creates a new range spanning from start to end.
	pub fn between(start: Point, end: Point) -> Self {
		Self { start, end }
	}

	/// Creates a single-line range (one where start and end are on the same
	/// line, but columns are different).
	pub fn within_line(line: usize, start_col: usize, end_col: usize) -> Self {
		Self {
			start: Point::at(line, start_col),
			end: Point::at(line, end_col),
		}
	}

	/// Creates a collapsed range (one where start and end points are equal).
	pub fn collapsed(line: usize, col: usize) -> Self {
		Self {
			start: Point::at(line, col),
			end: Point::at(line, col),
		}
	}

	pub fn is_collapsed(&self) -> bool {
		self.start.line == self.end.line && self.start.col == self.end.col
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Point {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}:{}", self.line, self.col)
	}
}

#[cfg(debug_assertions)]
impl std::fmt::Debug for Range {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{:#?}-{:#?}", self.start, self.end)
	}
}
