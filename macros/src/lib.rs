#[macro_export]
macro_rules! dump {
	($expr:expr) => {
		if cfg!(debug_assertions) {
			if std::env::var("PLUMA_DUMP").is_ok() {
				println!("{:#?}", $expr);
			}
		}
	};

	($label:literal, $expr:expr) => {
		if cfg!(debug_assertions) {
			if std::env::var("PLUMA_DUMP").is_ok() {
				println!("{}: {:#?}", $label, $expr);
			}
		}
	};
}
