use compiler::*;

use crate::printing::*;

pub(crate) fn analyze_command(entry_path: String) {
	let mut compiler = match Compiler::from_entry_path(entry_path) {
		Ok(c) => c,
		Err(diagnostics) => {
			print_diagnostics(diagnostics);
			std::process::exit(1);
		}
	};

	if let Err(diagnostics) = compiler.check() {
		print_diagnostics(diagnostics);
		std::process::exit(1);
	}
	let entry_name = compiler.entry_modules.first().cloned().unwrap_or_default();
	let module = compiler.modules.get(&entry_name).unwrap();
	println!("{:#?}", module);
}
