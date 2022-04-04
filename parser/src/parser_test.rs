use crate::{Parser, Tokenizer};

/// These tests cover the basic functionality of the parser.
///
/// See the snapshots/ directory in this crate for the snapshot output. The files tend
/// to be named like `{crate}__{file}__{test_name}.snap` - for example, see
/// `parser__parser_test__hello_world.snap`.

macro_rules! snapshot {
	($test_name:ident, $raw_source:literal) => {
		#[test]
		fn $test_name() {
			let source = $raw_source;
			let bytes = source.as_bytes().to_vec();
			let tokenizer = Tokenizer::from_source(&bytes);
			let mut parser = Parser::new(&bytes, tokenizer);
			let result = parser.parse_module();

			insta::assert_snapshot!(format!("{}\n\n{:#?}", source, result));
		}
	};
}

snapshot!(hello_world, "hello world");
