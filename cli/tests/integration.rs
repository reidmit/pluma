use std::path::Path;
use std::process::Command;

macro_rules! run_command {
  ($cmd:literal) => {
    run_command!($cmd, Vec::<String>::new())
  };

  ($cmd:literal, $args:expr) => {{
    let path = std::env::current_dir()
      .expect("failed to get current dir")
      .join(Path::new("../target/debug"))
      .join(Path::new($cmd));

    let working_dir = path.to_str().expect("path is not valid str");

    let result = Command::new(working_dir)
      .args($args)
      .output()
      .expect("failed to execute process");

    let status = match result.status.code() {
      Some(int_value) => int_value,
      None => panic!("no exit code for process"),
    };

    let stdout = String::from_utf8(result.stdout).expect("stdout is not utf-8");
    let stderr = String::from_utf8(result.stderr).expect("stderr is not utf-8");

    (status, stdout, stderr)
  }};
}

#[test]
fn integration_run_command_basic() {
  let (status, _, _) = run_command!("pluma");

  assert_eq!(status, 0);
}

#[test]
fn integration_run_command_version() {
  let (status, stdout, stderr) = run_command!("pluma", &["version"]);

  assert_eq!(stdout, "pluma version 0.1.0\n".to_owned());
  assert_eq!(stderr, "".to_owned());
  assert_eq!(status, 0);
}
