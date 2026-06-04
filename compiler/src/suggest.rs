// "Did you mean?" support. Given a misspelled name and a pool of candidates
// that were actually in scope, find the closest one by edit distance — but only
// suggest it when it's close enough to plausibly be a typo.

// Returns the candidate closest to `target` within a typo-sized edit distance,
// or `None` if nothing is close. The threshold scales with the target length
// (`max(1, len/3)`) so short names need a near-exact match while longer names
// tolerate a couple of slips. Ties break deterministically (smaller distance,
// then shorter candidate, then lexicographic) so snapshots stay stable.
pub fn closest<I, S>(target: &str, candidates: I) -> Option<String>
where
	I: IntoIterator<Item = S>,
	S: AsRef<str>,
{
	let threshold = (target.chars().count() / 3).max(1);

	let mut best: Option<(usize, String)> = None;
	for candidate in candidates {
		let candidate = candidate.as_ref();
		if candidate == target {
			// The name is in scope verbatim — not a typo we can help with.
			continue;
		}
		let distance = edit_distance(target, candidate);
		if distance > threshold {
			continue;
		}
		let better = match &best {
			None => true,
			Some((best_distance, best_candidate)) => {
				(distance, candidate.len(), candidate)
					< (
						*best_distance,
						best_candidate.len(),
						best_candidate.as_str(),
					)
			}
		};
		if better {
			best = Some((distance, candidate.to_string()));
		}
	}

	best.map(|(_, candidate)| candidate)
}

// Classic Levenshtein distance (insert/delete/substitute), over Unicode scalar
// values, using two rolling rows.
fn edit_distance(a: &str, b: &str) -> usize {
	let a: Vec<char> = a.chars().collect();
	let b: Vec<char> = b.chars().collect();

	if a.is_empty() {
		return b.len();
	}
	if b.is_empty() {
		return a.len();
	}

	let mut prev: Vec<usize> = (0..=b.len()).collect();
	let mut curr: Vec<usize> = vec![0; b.len() + 1];

	for (i, &ac) in a.iter().enumerate() {
		curr[0] = i + 1;
		for (j, &bc) in b.iter().enumerate() {
			let cost = if ac == bc { 0 } else { 1 };
			curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
		}
		std::mem::swap(&mut prev, &mut curr);
	}

	prev[b.len()]
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn suggests_close_typo() {
		let candidates = ["length", "map", "filter", "fold"];
		assert_eq!(closest("lenght", candidates), Some("length".to_string()));
		assert_eq!(closest("flter", candidates), Some("filter".to_string()));
	}

	#[test]
	fn rejects_distant_names() {
		let candidates = ["length", "map", "filter"];
		assert_eq!(closest("xyzzy", candidates), None);
	}

	#[test]
	fn ignores_exact_match() {
		let candidates = ["length", "lenght"];
		// `length` is present verbatim, so it isn't itself a suggestion; the
		// near-miss `lenght` is the only candidate within range.
		assert_eq!(closest("length", candidates), Some("lenght".to_string()));
	}

	#[test]
	fn short_names_need_near_exact() {
		// threshold for a 2-char name is 1.
		assert_eq!(closest("ab", ["xy"]), None);
		assert_eq!(closest("ab", ["ax"]), Some("ax".to_string()));
	}

	#[test]
	fn ties_break_deterministically() {
		// Both "axc" and "abx" are distance 1 from "abc"; the lexicographically
		// smaller candidate wins.
		assert_eq!(closest("abc", ["axc", "abx"]), Some("abx".to_string()));
	}
}
