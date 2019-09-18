#[macro_export]
#[cfg(debug_assertions)]
macro_rules! debug {
  ($($arg:tt)*) => {{
    println!($($arg)*);
  }};
}

#[macro_export]
#[cfg(not(debug_assertions))]
macro_rules! debug {
  ($($arg:tt)*) => {};
}