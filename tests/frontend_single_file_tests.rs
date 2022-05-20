use pencil::*;

/// These tests are meant to test the "frontend" of the compiler, which
/// mostly means parsing and type analysis. They use input files located
/// in the `tests/inputs` directory, and they write snapshots to the
/// `tests/snapshots` directory.
///
/// For example, test `hello_world` in this file has a snapshot at
/// tests/snapshots/frontend_single_file_tests__hello_world.snap

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
  snapshot_test!("tests/inputs/helloWorld.pa");
}

#[test]
fn identity_fun() {
  snapshot_test!("tests/inputs/identity.pa");
}
