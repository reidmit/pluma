#[cfg_attr(debug_assertions, derive(Debug))]
pub struct DocItem {
  pub name: String,
  pub comment_ranges: Vec<(usize, usize)>,
  pub kind: DocItemKind,
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum DocItemKind {
  Alias,
  Const,
  Def,
  Enum,
  Struct { fields: Vec<DocItem> },
}
