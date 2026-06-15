//! A dependency-free source watcher shared by the commands that re-run on save
//! (`pluma dev`, `pluma test --watch`). It polls a cheap change fingerprint
//! rather than subscribing to OS file events — a full rebuild is tens of ms, so
//! a quarter-second poll is plenty responsive and needs no platform-specific
//! plumbing.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// How often the watch loop re-scans for changes. A rebuild is far cheaper than
/// a human's edit-save cadence, so this just needs to feel instant.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// A cheap change fingerprint: (count of `*.pa` files, latest mtime among them).
/// Comparing this across polls catches edits, additions, and deletions. Hidden
/// directories (`.git`, `target`, …) are skipped.
pub(crate) fn scan(root: &Path) -> (usize, Option<SystemTime>) {
	fn walk(dir: &Path, count: &mut usize, latest: &mut Option<SystemTime>) {
		let entries = match std::fs::read_dir(dir) {
			Ok(e) => e,
			Err(_) => return,
		};
		for entry in entries.flatten() {
			let name = entry.file_name();
			let name = name.to_string_lossy();
			if name.starts_with('.') || name == "target" {
				continue;
			}
			let file_type = match entry.file_type() {
				Ok(t) => t,
				Err(_) => continue,
			};
			let path = entry.path();
			if file_type.is_dir() {
				walk(&path, count, latest);
			} else if name.ends_with(".pa") {
				*count += 1;
				if let Ok(m) = entry.metadata().and_then(|m| m.modified()) {
					if latest.map_or(true, |b| m > b) {
						*latest = Some(m);
					}
				}
			}
		}
	}
	let mut count = 0;
	let mut latest = None;
	walk(root, &mut count, &mut latest);
	(count, latest)
}
