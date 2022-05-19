use pencil::*;

/// These tests are meant to test the "frontend" of the compiler, which
/// mostly means parsing and type analysis.

macro_rules! snapshot_test {
  ($path: literal) => {
    let mut compiler = Compiler::from_entry_path($path.to_string()).unwrap();

    let result = match compiler.check() {
      Ok(_) => Ok(
        compiler
          .modules
          .get(&compiler.entry_module_name)
          .unwrap()
          .ast
          .as_ref(),
      ),
      Err(diagnostics) => Err(diagnostics),
    };

    insta::assert_debug_snapshot!(result);
  };
}

#[test]
fn hello_world() {
  snapshot_test!("tests/inputs/hello-world.pa");
}
