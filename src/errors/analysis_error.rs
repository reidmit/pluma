use std::fmt;

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct AnalysisError {
	pub pos: (usize, usize),
	pub kind: AnalysisErrorKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum AnalysisErrorKind {}

impl fmt::Display for AnalysisError {
	fn fmt(&self, _f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		// use AnalysisErrorKind::*;

		match &self.kind {
			_ => Ok(()),
		}
	}
}
