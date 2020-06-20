use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::path::PathBuf;

pub struct Repl {
  pub history_dir: Option<PathBuf>,
  rl: Editor<()>,
}

impl Repl {
  pub fn new(history_dir: Option<PathBuf>) -> Self {
    let rl = Editor::<()>::new();

    Repl { history_dir, rl }
  }

  pub fn start(&mut self) {
    if let Some(dir) = &self.history_dir {
      if self.rl.load_history(&dir.join(".repl_history")).is_err() {
        println!("No previous history.");
      }
    }

    let mut last_ctrl_c = false;

    loop {
      let readline = self.rl.readline("\n> ");

      match readline {
        Ok(line) => {
          last_ctrl_c = false;

          self.rl.add_history_entry(line.as_str());

          if line.starts_with(".") {
            if self.handle_keyword(&line) {
              break;
            } else {
              continue;
            }
          }

          println!("Line: {}", line);
        }

        Err(ReadlineError::Interrupted) => {
          if last_ctrl_c {
            println!("Exiting.");
            break;
          }

          last_ctrl_c = true;
        }

        Err(ReadlineError::Eof) => {
          println!("Exiting.");
          break;
        }

        Err(err) => {
          println!("Error: {}", err);
          break;
        }
      }
    }

    if let Some(dir) = &self.history_dir {
      self.rl.save_history(&dir.join(".repl_history")).unwrap();
    }
  }

  fn handle_keyword(&mut self, line: &String) -> bool {
    let keyword = self.extract_repl_keyword(line.as_bytes());

    match &keyword[..] {
      "exit" => {
        println!("Exiting.");
        return true;
      }

      "help" => {
        println!("Available commands:");
        println!("  .clear   Clear REPL history");
        println!("  .exit    Exit REPL (also Ctrl-D)");
        println!("  .help    Print this help text");
      }

      "clear" => {
        self.rl.clear_history();
        println!("Cleared REPL history.");
      }

      _ => {
        println!("Unknown REPL command: '.{}'", keyword);
        println!("For help, try '.help'.")
      }
    }

    return false;
  }

  fn extract_repl_keyword(&mut self, given: &[u8]) -> String {
    let mut i = 1;

    while i < given.len() {
      let byte = given[i];

      match byte {
        _ if byte.is_ascii_whitespace() => break,
        _ => i += 1,
      }
    }

    String::from_utf8(given[1..i].to_vec()).unwrap()
  }
}
