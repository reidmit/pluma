use crate::colors;
use pluma_compiler::{BINARY_NAME, VERSION};
use rustyline::error::ReadlineError;
use rustyline::Editor;

pub fn run() {
  println!(
    "{} {} (version {})",
    colors::bold(BINARY_NAME),
    colors::bold("repl"),
    VERSION
  );
  println!("Use Ctrl-D or type '.exit' to quit.");
  println!("Type '.help' for more.");

  let mut rl = Editor::<()>::new();
  if rl.load_history("history.txt").is_err() {
    println!("No previous history.");
  }

  let mut last_ctrl_c = false;

  loop {
    let readline = rl.readline(&colors::bold_dim("\n> ")[..]);

    match readline {
      Ok(line) => {
        last_ctrl_c = false;

        rl.add_history_entry(line.as_str());

        if line.starts_with(".") {
          if handle_keyword(&line) {
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
        println!("Error: {:?}", err);
        break;
      }
    }
  }

  rl.save_history("history.txt").unwrap();
}

fn handle_keyword(line: &String) -> bool {
  let keyword = extract_repl_keyword(line.as_bytes());

  match keyword {
    "exit" => {
      println!("Exiting.");
      return true;
    }

    "help" => {
      println!("Helping...");
    }

    _ => {
      println!(
        "Unknown REPL command: '.{}'. For help, try '.help'.",
        keyword
      );
    }
  }

  return false;
}

fn extract_repl_keyword(given: &[u8]) -> &str {
  let mut i = 1;

  while i < given.len() {
    let byte = given[i];

    match byte {
      _ if byte.is_ascii_whitespace() => break,
      _ => i += 1,
    }
  }

  std::str::from_utf8(&given[1..i]).unwrap()
}
