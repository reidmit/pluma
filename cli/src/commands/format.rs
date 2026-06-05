use crate::printing::*;

pub(crate) fn format_command(check: bool, paths: Vec<String>) {
	if paths.is_empty() {
		print_error("No path given. Expected a file path or `-` for stdin.");
		std::process::exit(1);
	}

	let mut any_changed = false;

	for path in &paths {
		if path == "-" {
			let mut input = Vec::new();
			if let Err(err) = std::io::Read::read_to_end(&mut std::io::stdin(), &mut input) {
				print_error(format!("Failed to read stdin: {}", err));
				std::process::exit(1);
			}
			match formatter::format_source(&input) {
				Ok(out) => {
					if check {
						if out.as_bytes() != input.as_slice() {
							any_changed = true;
						}
					} else {
						print!("{}", out);
					}
				}
				Err(diagnostics) => {
					print_diagnostics(diagnostics);
					std::process::exit(1);
				}
			}
			continue;
		}

		let bytes = match std::fs::read(path) {
			Ok(b) => b,
			Err(err) => {
				print_error(format!("Could not read `{}`: {}", path, err));
				std::process::exit(1);
			}
		};

		match formatter::format_source(&bytes) {
			Ok(out) => {
				let changed = out.as_bytes() != bytes.as_slice();
				if check {
					if changed {
						any_changed = true;
						eprintln!("would reformat {}", path);
					}
				} else if changed {
					if let Err(err) = std::fs::write(path, out.as_bytes()) {
						print_error(format!("Could not write `{}`: {}", path, err));
						std::process::exit(1);
					}
				}
			}
			Err(_diagnostics) => {
				// Skip unparseable files rather than aborting — the user may
				// be formatting a batch that includes intentionally-broken
				// fixtures (e.g. analyzer error tests).
				eprintln!("skipping {} (parse error)", path);
			}
		}
	}

	if check && any_changed {
		std::process::exit(1);
	}
}
