use crate::Tokenizer;

/// These tests cover the basic functionality of the tokenizer. I don't really expect
/// the tokenizer to be used on its own, but more granular unit tests will make it
/// easier to figure out where things are going wrong if there are bugs.
///
/// See the snapshots/ directory in this crate for the snapshot output. The files tend
/// to be named like `{crate}__{file}__{test_name}.snap` - for example, see
/// `parser__tokenizer_test__identifiers.snap`.

macro_rules! snapshot {
	($test_name:ident, $raw_source:literal) => {
		#[test]
		fn $test_name() {
			let source = $raw_source;
			let bytes = source.as_bytes().to_vec();
			let mut tokens = Vec::new();
			let mut tokenizer = Tokenizer::from_source(&bytes);
			while let Some(token) = tokenizer.next() {
				tokens.push(token);
			}

			insta::assert_snapshot!(format!("{}\n\n{:#?}", source, tokens));
		}
	};
}

snapshot!(numbers, "1 2 3 4");

snapshot!(identifiers, "hello world");

snapshot!(string_literals, "\"a\" \"bb\" \"ccc\"");

snapshot!(string_interpolations, "\"before $(some-variable) after\"");
