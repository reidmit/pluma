enum Command {
  Run,
  Help,
  Version,
}

pub struct CLIOptions {
  command: Command,
}