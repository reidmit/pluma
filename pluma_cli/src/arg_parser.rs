use crate::command_error::{CommandError, CommandErrorKind};
use crate::command_info::*;
use std::collections::{HashMap, HashSet};

#[cfg_attr(debug_assertions, derive(Debug))]
pub struct ParsedArgs {
  command_name: String,
  positional_args: Vec<String>,
  bool_flags: HashSet<String>,
  single_value_flags: HashMap<String, String>,
  multi_value_flags: HashMap<String, Vec<String>>,
}

impl ParsedArgs {
  pub fn get_positional_arg(&self, index: usize) -> Option<String> {
    match self.positional_args.get(index) {
      None => None,
      Some(arg) => Some(arg.clone()),
    }
  }

  pub fn is_flag_present(&self, flag: &'static str) -> bool {
    match self.bool_flags.get(flag) {
      None => false,
      _ => true,
    }
  }

  pub fn get_flag_value(&self, flag: &'static str) -> Option<String> {
    match self.single_value_flags.get(flag) {
      Some(val) => Some(val.clone()),
      None => None,
    }
  }
}

pub fn find_subcommand() -> (Option<String>, bool, Vec<String>) {
  let mut subcommand = None;
  let mut is_help_requested = false;
  let mut other_args = Vec::new();

  for arg in std::env::args().skip(1) {
    if subcommand.is_none() && !arg.starts_with("-") {
      subcommand = Some(arg);
    } else {
      if arg == "-h" || arg == "--help" {
        is_help_requested = true;
      }

      other_args.push(arg);
    }
  }

  (subcommand, is_help_requested, other_args)
}

pub fn parse_args_for_command(
  args: Vec<String>,
  cmd: CommandInfo,
) -> Result<ParsedArgs, CommandError> {
  let mut parsed = ParsedArgs {
    command_name: cmd.name.to_owned(),
    positional_args: Vec::new(),
    single_value_flags: HashMap::new(),
    multi_value_flags: HashMap::new(),
    bool_flags: HashSet::new(),
  };

  let mut allowed_flags = HashMap::new();
  let mut allowed_flag_aliases = HashMap::new();
  let positional_arg_count = match cmd.args {
    Some(args) => args.len(),
    None => 0,
  };

  if let Some(flags) = cmd.flags {
    for flag in flags {
      let long_name = flag.long_name.to_owned();

      if let Some(short_name) = flag.short_name {
        allowed_flag_aliases.insert(short_name, long_name.clone());
      }

      allowed_flags.insert(long_name, flag);
    }
  }

  let mut i = 0;

  while i < args.len() {
    let arg = &args[i];

    if arg.starts_with("-") {
      let is_long_flag = arg.len() > 1 && arg.bytes().nth(1).unwrap() == b'-';
      let name_start = if is_long_flag { 2 } else { 1 };
      let flag_name = arg[name_start..].to_owned();

      let allowed_flag = allowed_flags.get(&flag_name[..]).or_else(|| {
        match allowed_flag_aliases.get(&flag_name[..]) {
          Some(full_name) => allowed_flags.get(&full_name[..]),
          None => None,
        }
      });

      if allowed_flag.is_none() {
        return Err(CommandError {
          command: cmd.name.to_owned(),
          kind: CommandErrorKind::UnexpectedFlag(flag_name),
        });
      }

      let flag_info = allowed_flag.unwrap();
      let flag_name = flag_info.long_name.to_owned();

      if flag_info.style == FlagStyle::Boolean {
        if parsed.bool_flags.contains(&flag_name) {
          return Err(CommandError {
            command: cmd.name.to_owned(),
            kind: CommandErrorKind::DuplicateValueForFlag(flag_name),
          });
        }

        parsed.bool_flags.insert(flag_name);
        i += 1;
        continue;
      }

      let next_value = args.get(i + 1);

      if next_value.is_none() || next_value.unwrap().starts_with("-") {
        return Err(CommandError {
          command: cmd.name.to_owned(),
          kind: CommandErrorKind::MissingValueForFlag(flag_name),
        });
      }

      let next_value = next_value.unwrap().clone();

      if flag_info.style == FlagStyle::SingleValue {
        if parsed.single_value_flags.contains_key(&flag_name) {
          return Err(CommandError {
            command: cmd.name.to_owned(),
            kind: CommandErrorKind::DuplicateValueForFlag(flag_name),
          });
        }

        if !flag_info.supports_value(&next_value) {
          return Err(CommandError {
            command: cmd.name.to_owned(),
            kind: CommandErrorKind::InvalidValueForFlag(flag_name, next_value),
          });
        }

        parsed
          .single_value_flags
          .insert(flag_name, next_value.clone());

        i += 1;
        continue;
      }

      if flag_info.style == FlagStyle::MultipleValue {
        if let Some(values) = parsed.multi_value_flags.get_mut(&flag_name) {
          values.push(next_value.clone());
        } else {
          parsed
            .multi_value_flags
            .insert(flag_name, vec![next_value.clone()]);
        }
      }

      i += 1;
      continue;
    }

    if parsed.positional_args.len() >= positional_arg_count {
      return Err(CommandError {
        command: cmd.name.to_owned(),
        kind: CommandErrorKind::UnexpectedArgument(arg.clone()),
      });
    }

    parsed.positional_args.push(arg.clone());

    i += 1;
  }

  Ok(parsed)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn bool_flag() {
    let cmd = CommandInfo::new("testcmd", "just for testing")
      .flags(vec![Flag::with_names("bool-flag", "b")]);

    let args = vec!["--bool-flag".to_owned()];
    let parsed = parse_args_for_command(args, cmd).expect("should be valid");

    assert_eq!(parsed.is_flag_present("bool-flag"), true);
    assert_eq!(parsed.is_flag_present("something-else"), false);
  }

  #[test]
  fn bool_flag_not_provided() {
    let cmd = CommandInfo::new("testcmd", "just for testing")
      .flags(vec![Flag::with_names("bool-flag", "b")]);

    let args = vec![];
    let parsed = parse_args_for_command(args, cmd).expect("should be valid");

    assert_eq!(parsed.is_flag_present("bool-flag"), false);
  }
}

// pub fn parse_args(args: Vec<String>) -> ParsedArgs {
//   let mut parsed = ParsedArgs {
//     positional_args: Vec::new(),
//     flags: HashMap::new(),
//     additional_args: Vec::new(),
//     retrieved_args: HashSet::new(),
//     retrieved_flags: HashSet::new(),
//   };

//   let mut i = 0;
//   let mut found_additional_separator = false;

//   while i < args.len() {
//     let arg = &args[i];

//     if found_additional_separator {
//       parsed.additional_args.push(arg.clone());
//       i += 1;
//       continue;
//     }

//     if arg.starts_with("-") {
//       if arg == "--" {
//         found_additional_separator = true;
//         i += 1;
//         continue;
//       }

//       let is_long_flag = arg.len() > 1 && arg.bytes().nth(1).unwrap() == b'-';
//       let name_start = if is_long_flag { 2 } else { 1 };
//       let arg_name = arg[name_start..].to_owned();

//       let next_value = args.get(i + 1);

//       if next_value.is_none() || next_value.unwrap().starts_with("-") {
//         if !parsed.flags.contains_key(&arg_name) {
//           parsed.flags.insert(arg_name, vec![]);
//         }

//         i += 1;
//         continue;
//       }

//       if parsed.flags.contains_key(&arg_name) {
//         let entry = parsed.flags.get_mut(&arg_name).unwrap();
//         entry.push(next_value.unwrap().clone());
//       } else {
//         parsed
//           .flags
//           .insert(arg_name, vec![next_value.unwrap().clone()]);
//       }

//       i += 2;
//       continue;
//     }

//     parsed.positional_args.push(arg.clone());

//     i += 1;
//   }

//   parsed
// }
