use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::fmt;

#[derive(Debug)]
pub struct ImportChain {
  pub entries: Vec<String>,
  seen: HashSet<u64>
}

impl ImportChain {
  pub fn new() -> Self {
    ImportChain {
      entries: Vec::new(),
      seen: HashSet::new(),
    }
  }

  pub fn contains(&self, key: String) -> bool {
    self.seen.contains(&hash_key(&key))
  }

  pub fn add(&mut self, path: String) {
    let key = hash_key(&path);

    if self.seen.contains(&key) {
      panic!("todo");
    }

    self.seen.insert(key);
    self.entries.push(path);
  }
}

impl Clone for ImportChain {
  fn clone(&self) -> ImportChain {
    let mut new = ImportChain::new();

    for entry in self.entries.clone() {
      new.add(entry);
    }

    new
  }
}

impl fmt::Display for ImportChain {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    write!(f, "{:#?}", self.entries)
  }
}

fn hash_key(key: &String) -> u64 {
  let mut hasher = DefaultHasher::new();
  key.hash(&mut hasher);
  hasher.finish()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_simple() {
    let mut p = ImportChain::new();
    assert_eq!(p.contains("main.pa".to_owned()), false);

    p.add("main.pa".to_owned());
    assert_eq!(p.contains("main.pa".to_owned()), true);
    assert_eq!(p.contains("other.pa".to_owned()), false);
  }

  #[test]
  fn test_clone() {
    let mut p = ImportChain::new();
    p.add("main.pa".to_owned());

    let p2 = p.clone();
    assert_eq!(p2.contains("main.pa".to_owned()), true);
    assert_eq!(p2.contains("other.pa".to_owned()), false);
  }
}