use pencil::*;

fn main() {
  let entry_path = match std::env::args().nth(1) {
    Some(path) => path,
    None => panic!("no arg given"),
  };

  let mut compiler = match Compiler::from_entry_path(entry_path) {
    Ok(c) => c,
    Err(diagnostics) => {
      print_diagnostics(diagnostics);
      std::process::exit(1);
    }
  };

  match compiler.check() {
    Ok(_) => {
      println!("Check succeeded without errors!");
    }

    Err(diagnostics) => {
      print_diagnostics(diagnostics);
      std::process::exit(1);
    }
  }
}
