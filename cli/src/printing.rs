use crate::colors;
use compiler::*;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

pub fn print_error<E: std::fmt::Display>(message: E) {
	print_diagnostics(vec![Diagnostic::error(message)])
}

pub fn print_diagnostics(diagnostics: Vec<Diagnostic>) {
	let mut first = true;

	for diagnostic in diagnostics {
		if !first {
			eprintln!("")
		}

		let is_error = diagnostic.is_error();

		eprintln!(
			"{} {}",
			if is_error {
				colors::bold_red("Error:")
			} else {
				colors::bold_yellow("Warning:")
			},
			diagnostic.message
		);

		if diagnostic.module_path.is_none() {
			continue;
		}

		let mut module_path = diagnostic.module_path.unwrap();
		let cwd = std::env::current_dir().unwrap_or(PathBuf::from(""));
		if module_path.starts_with(cwd) {
			module_path = module_path
				.strip_prefix(std::env::current_dir().unwrap())
				.unwrap()
				.to_path_buf();
		}

		if let Some(Range { start, end }) = diagnostic.range {
			let file = File::open(&module_path).unwrap();

			let mut relevant_lines: Vec<(usize, String)> = Vec::new();

			let mut current_line = 0;
			for text in BufReader::new(file).lines().flatten() {
				if current_line >= start.line && current_line <= end.line {
					relevant_lines.push((current_line, text.replace("\t", " ")));
				} else if current_line > end.line {
					break;
				}

				current_line += 1;
			}

			for (line_index, text) in relevant_lines {
				eprintln!(
					"\n{} {} {}",
					if is_error {
						colors::bold_red(">")
					} else {
						colors::bold_yellow(">")
					},
					// add 1 for user-friendly display:
					colors::bold_dim(format!("{}|", line_index + 1).as_str()),
					text
				);

				if line_index == start.line && line_index == end.line {
					let up_arrows = "^".repeat(end.col - start.col).to_string();

					// calculate left padding for left arrow/line number/indent:
					let prefix_width = 4 + line_index.to_string().len();

					eprintln!(
						"{}{}",
						" ".repeat(prefix_width + start.col),
						if is_error {
							colors::bold_red(&up_arrows)
						} else {
							colors::bold_yellow(&up_arrows)
						}
					);
				} else if line_index == end.line && line_index != start.line {
					eprintln!("");
				}
			}

			eprintln!(
				"{}",
				colors::dim(
					format!(
						"{}:{}:{}",
						module_path.to_str().unwrap(),
						start.line + 1, // add 1 for user-friendly display
						start.col + 1   // add 1 for user-friendly display
					)
					.as_str()
				)
			);
		}

		first = false;
	}
}
