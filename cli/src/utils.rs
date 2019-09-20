use term_size;

pub fn get_terminal_width() -> usize {
  if let Some((width, _)) = term_size::dimensions() {
    return width;
  } else {
    return 80;
  }
}