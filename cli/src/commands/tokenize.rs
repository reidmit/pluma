use compiler::*;

use crate::printing::*;

pub(crate) fn tokenize_command(entry_path: String) {
	let mut compiler = match Compiler::from_entry_path(entry_path) {
		Ok(c) => c,
		Err(diagnostics) => {
			print_diagnostics(diagnostics);
			std::process::exit(1);
		}
	};

	match compiler.tokenize() {
		Ok(tokens) => {
			for token in tokens {
				println!("{:?}", token);
			}
		}

		Err(diagnostics) => {
			print_diagnostics(diagnostics);
			std::process::exit(1);
		}
	}
}
