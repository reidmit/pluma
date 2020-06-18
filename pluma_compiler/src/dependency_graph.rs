use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::hash::Hasher;

#[cfg_attr(debug_assertions, derive(Debug))]
pub enum TopologicalSort {
  Sorted(Vec<String>),
  Cycle(Vec<String>),
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct DependencyGraph {
  entry_vertex: String,
  vertices: HashSet<String>,
  edges: HashMap<String, HashSet<String>>,
  cached_sort: Option<TopologicalSort>,
}

impl DependencyGraph {
  pub fn new(entry: String) -> Self {
    let mut vertices = HashSet::new();
    vertices.insert(entry.to_string());
    let mut edges = HashMap::new();
    edges.insert(entry.to_string(), HashSet::new());

    DependencyGraph {
      entry_vertex: entry,
      vertices,
      edges,
      cached_sort: None,
    }
  }

  pub fn add_edge(&mut self, imported: String, importer: String) {
    self.cached_sort = None;

    self.vertices.insert(imported.to_owned());
    self.vertices.insert(importer.to_owned());

    let entry = self.edges.entry(importer).or_insert(HashSet::new());
    entry.insert(imported.to_owned());

    self.edges.entry(imported).or_insert(HashSet::new());
  }

  pub fn sort(&mut self) -> &TopologicalSort {
    if let None = &self.cached_sort {
      self.do_sort();
    }

    match &self.cached_sort {
      Some(sort) => sort,
      None => unreachable!(),
    }
  }

  fn do_sort(&mut self) {
    let mut sorted = Vec::with_capacity(self.vertices.len());

    let mut in_degrees = HashMap::with_capacity(self.edges.len());
    for vertex in &self.vertices {
      in_degrees.insert(vertex, 0);
    }

    for (_, edges) in &self.edges {
      for vertex in edges {
        let entry = in_degrees.entry(vertex).or_insert(0);
        *entry += 1;
      }
    }

    let mut queue = VecDeque::new();
    for from in &self.vertices {
      if *in_degrees.get(from).unwrap() == 0 {
        queue.push_back(from.to_string());
      }
    }

    let mut visited_count = 0;
    while !queue.is_empty() {
      let vertex = queue.pop_front().unwrap();
      let key = vertex.to_string();
      sorted.push(vertex);

      for edge in self.edges.get(&key).unwrap().into_iter() {
        if let Some(entry) = in_degrees.get_mut(&edge) {
          *entry -= 1;

          if *entry == 0 {
            queue.push_back(edge.to_string());
          }
        }
      }

      visited_count += 1;
    }

    let result = if visited_count != self.edges.len() {
      TopologicalSort::Cycle(self.find_cycle_from_entry())
    } else {
      sorted.reverse();
      TopologicalSort::Sorted(sorted)
    };

    self.cached_sort = Some(result);
  }

  fn find_cycle_from_entry(&self) -> Vec<String> {
    let chain = self.build_path(&self.entry_vertex, ImportChain::new());
    chain.unwrap_err().cyclic_entries()
  }

  fn build_path(&self, vertex: &String, import_chain: ImportChain) -> Result<(), ImportChain> {
    if import_chain.contains(vertex.to_owned()) {
      return Err(import_chain.extend(vertex.to_owned()));
    }

    for edge in self.edges.get(vertex).unwrap() {
      let extended_import_chain = import_chain.extend(vertex.to_owned());
      self.build_path(edge, extended_import_chain)?;
    }

    Ok(())
  }
}

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ImportChain {
  pub entries: Vec<String>,
  seen: HashSet<u64>,
}

impl ImportChain {
  pub fn new() -> Self {
    ImportChain {
      entries: Vec::new(),
      seen: HashSet::new(),
    }
  }

  pub fn extend(&self, path: String) -> Self {
    let mut new_chain = self.clone();
    new_chain.add(path);
    new_chain
  }

  pub fn contains(&self, key: String) -> bool {
    self.seen.contains(&hash_key(&key))
  }

  pub fn add(&mut self, path: String) {
    let key = hash_key(&path);

    self.seen.insert(key);
    self.entries.push(path);
  }

  pub fn cyclic_entries(&self) -> Vec<String> {
    let last = self.entries.last().unwrap();

    let mut i = 0;
    while self.entries.get(i).unwrap() != last {
      i += 1;
    }

    self.entries[i..].to_owned()
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

fn hash_key(key: &String) -> u64 {
  let mut hasher = DefaultHasher::new();
  key.hash(&mut hasher);
  hasher.finish()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_insert() {
    let mut g = DependencyGraph::new("a".to_owned());
    g.add_edge("a".to_owned(), "b".to_owned());
    g.add_edge("b".to_owned(), "c".to_owned());

    assert!(g
      .edges
      .get(&"b".to_owned())
      .unwrap()
      .contains(&"a".to_owned()));
    assert!(g
      .edges
      .get(&"c".to_owned())
      .unwrap()
      .contains(&"b".to_owned()));
  }

  #[test]
  fn test_sort() {
    let mut g = DependencyGraph::new("a".to_owned());
    g.add_edge("c".to_owned(), "d".to_owned());
    g.add_edge("a".to_owned(), "b".to_owned());
    g.add_edge("b".to_owned(), "c".to_owned());

    match g.sort() {
      TopologicalSort::Sorted(sorted) => assert_eq!(
        sorted.to_owned(),
        vec![
          "a".to_owned(),
          "b".to_owned(),
          "c".to_owned(),
          "d".to_owned(),
        ]
      ),
      TopologicalSort::Cycle(..) => panic!("Unexpected cycle"),
    }
  }

  #[test]
  fn test_sort_no_edges() {
    let mut g = DependencyGraph::new("a".to_owned());

    match g.sort() {
      TopologicalSort::Sorted(sorted) => assert_eq!(sorted.to_owned(), vec!["a".to_owned(),]),
      TopologicalSort::Cycle(..) => panic!("Unexpected cycle"),
    }
  }

  #[test]
  fn test_sort_2() {
    let mut g = DependencyGraph::new("a".to_owned());
    g.add_edge("a".to_owned(), "b".to_owned());
    g.add_edge("b".to_owned(), "c".to_owned());
    g.add_edge("a".to_owned(), "c".to_owned());

    match g.sort() {
      TopologicalSort::Sorted(sorted) => assert_eq!(
        sorted.to_owned(),
        vec!["a".to_owned(), "b".to_owned(), "c".to_owned(),]
      ),
      TopologicalSort::Cycle(..) => panic!("Unexpected cycle"),
    }
  }

  #[test]
  fn test_cycle() {
    let mut g = DependencyGraph::new("b".to_owned());
    g.add_edge("b".to_owned(), "a".to_owned());
    g.add_edge("c".to_owned(), "b".to_owned());
    g.add_edge("a".to_owned(), "c".to_owned());

    match g.sort() {
      TopologicalSort::Sorted(..) => panic!("Unexpected sort"),
      TopologicalSort::Cycle(cycle) => assert_eq!(
        cycle.to_vec(),
        vec![
          "b".to_owned(),
          "c".to_owned(),
          "a".to_owned(),
          "b".to_owned(),
        ]
      ),
    }
  }

  #[test]
  fn test_cycle_in_long_chain() {
    let mut g = DependencyGraph::new("a".to_owned());
    g.add_edge("b".to_owned(), "a".to_owned());
    g.add_edge("c".to_owned(), "b".to_owned());
    g.add_edge("d".to_owned(), "c".to_owned());
    g.add_edge("e".to_owned(), "d".to_owned());
    g.add_edge("c".to_owned(), "e".to_owned());

    match g.sort() {
      TopologicalSort::Sorted(..) => panic!("Unexpected sort"),
      TopologicalSort::Cycle(cycle) => assert_eq!(
        cycle.to_vec(),
        vec![
          "c".to_owned(),
          "d".to_owned(),
          "e".to_owned(),
          "c".to_owned(),
        ]
      ),
    }
  }

  #[test]
  fn test_simple_import_chain() {
    let mut p = ImportChain::new();
    assert_eq!(p.contains("main".to_owned()), false);

    p.add("main".to_owned());
    assert_eq!(p.contains("main".to_owned()), true);
    assert_eq!(p.contains("other".to_owned()), false);
  }

  #[test]
  fn test_import_chain_clone() {
    let mut p = ImportChain::new();
    p.add("main".to_owned());

    let p2 = p.clone();
    assert_eq!(p2.contains("main".to_owned()), true);
    assert_eq!(p2.contains("other".to_owned()), false);
  }

  #[test]
  fn test_import_chain_cyclic_entries() {
    let mut p = ImportChain::new();
    p.add("a".to_owned());
    p.add("b".to_owned());
    p.add("c".to_owned());
    p.add("d".to_owned());
    p.add("b".to_owned());

    let cycle = p.cyclic_entries();
    assert_eq!(
      cycle,
      vec![
        "b".to_owned(),
        "c".to_owned(),
        "d".to_owned(),
        "b".to_owned(),
      ]
    );
  }
}
