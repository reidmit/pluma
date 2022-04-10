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

snapshot!(simple_assignment, "let x = 47.123");

snapshot!(regex_literal_sequence, "/ \"a\" \"b\" \"c\" /");

snapshot!(regex_literal_one_or_more, "/ \"a\" \"b\"+ \"c\" /");

snapshot!(regex_literal_one_or_zero, "/ \"a\" \"b\"? \"c\" /");

snapshot!(regex_literal_zero_or_more, "/ \"a\" \"b\"* \"c\" /");

snapshot!(regex_literal_alternation, "/ \"a\" (\"b\" | \"c\") \"d\" /");

snapshot!(regex_literal_exact_count, "/ \"a\" \"b\"{3} \"c\" /");

snapshot!(regex_literal_range_count, "/ \"a\" \"b\"{1,3} \"c\" /");

snapshot!(regex_literal_at_least_count, "/ \"a\" \"b\"{1,} \"c\" /");

snapshot!(regex_literal_at_most_count, "/ \"a\" \"b\"{,3} \"c\" /");

snapshot!(
	regex_literal_named_capture,
	"/ \"a\" <b_or_c:(\"b\" | \"c\")> \"d\" /"
);

snapshot!(let_block_no_params, "let b = { hello \"world\" }");
