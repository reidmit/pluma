//! CLI subcommand implementations — one module per `pluma` command. Each exposes
//! a single entry point invoked by the dispatcher in `main`; shared infrastructure
//! (diagnostics printing, the browser bundle) lives in the top-level modules.

pub(crate) mod build;
pub(crate) mod dev;
pub(crate) mod doc;
pub(crate) mod format;
pub(crate) mod lint;
pub(crate) mod run;
pub(crate) mod test;

#[cfg(debug_assertions)]
pub(crate) mod analyze;
#[cfg(debug_assertions)]
pub(crate) mod tokenize;

/// Expand directory arguments into the `.pa` files beneath them, recursively.
/// File paths (and `-` for stdin) pass through unchanged; a directory becomes
/// every `*.pa` file under it, sorted for stable output. Hidden directories
/// (anything starting with `.`) are skipped — `.git`, `.cargo`, etc. shouldn't
/// be scanned. Shared by `pluma format` and `pluma lint`.
pub(crate) fn expand_paths(paths: Vec<String>) -> Vec<String> {
	fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
		let entries = match std::fs::read_dir(dir) {
			Ok(e) => e,
			Err(_) => return,
		};
		for entry in entries.flatten() {
			let path = entry.path();
			let name = match path.file_name().and_then(|n| n.to_str()) {
				Some(n) => n,
				None => continue,
			};
			if name.starts_with('.') {
				continue;
			}
			let file_type = match entry.file_type() {
				Ok(t) => t,
				Err(_) => continue,
			};
			if file_type.is_dir() {
				walk(&path, out);
			} else if file_type.is_file() && name.ends_with(".pa") {
				out.push(path.to_string_lossy().into_owned());
			}
		}
	}

	let mut out = Vec::new();
	for path in paths {
		if path != "-" && std::path::Path::new(&path).is_dir() {
			let mut found = Vec::new();
			walk(std::path::Path::new(&path), &mut found);
			found.sort();
			out.extend(found);
		} else {
			out.push(path);
		}
	}
	out
}
