use crate::doc_item::*;
use pluma_ast::*;
use pluma_parser::*;
use pluma_visitor::*;
use std::collections::HashMap;

pub struct ItemCollector<'a> {
  pub items: HashMap<usize, DocItem>,
  comments: &'a CommentMap,
  line_break_positions: &'a Vec<Position>,
  last_processed_line: usize,
}

impl<'a> ItemCollector<'a> {
  pub fn new(comments: &'a CommentMap, line_break_positions: &'a Vec<Position>) -> Self {
    ItemCollector {
      items: HashMap::new(),
      comments,
      line_break_positions,
      last_processed_line: 0,
    }
  }

  fn comments_for_start_position(&mut self, pos: usize) -> Option<Vec<(usize, usize)>> {
    let mut line = self.line_for_coordinate(pos);

    if self.comments.contains_key(&line) {
      let mut associated_comments = Vec::new();

      while let Some(comment) = self.comments.get(&line) {
        let (start, end) = comment.get_position();
        associated_comments.push((start + 1, end));
        line -= 1;
      }

      return Some(associated_comments);
    }

    None
  }

  fn line_for_coordinate(&mut self, coord: usize) -> usize {
    for i in self.last_processed_line..self.line_break_positions.len() {
      let (line_break_start, _) = self.line_break_positions[i];

      if line_break_start > coord {
        self.last_processed_line = i;
        return i - 1;
      }
    }

    0
  }

  fn def_to_item_name(&self, node: &DefNode) -> String {
    "lol".to_owned()
  }
}

impl<'a> Visitor for ItemCollector<'a> {
  fn enter_module(&mut self, _node: &ModuleNode) {
    // println!("c: {:#?}", self.comments);
    // println!("lbs: {:#?}", self.line_break_positions);
  }

  fn enter_top_level_statement(&mut self, node: &TopLevelStatementNode) {
    match &node.kind {
      TopLevelStatementKind::Def(def) => {
        if def.visibility != ExportVisibility::Public {
          return;
        }

        let start_pos = def.pos.0;

        match self.comments_for_start_position(start_pos) {
          Some(comment_ranges) => {
            let name = self.def_to_item_name(def);

            self.items.insert(
              start_pos,
              DocItem {
                name,
                comment_ranges,
                kind: DocItemKind::Def,
              },
            );
          }
          None => {}
        }
      }

      _ => {}
    }
  }
}
