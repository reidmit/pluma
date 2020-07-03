use crate::colors;
use pluma_constants::*;
use std::fmt;

pub struct CommandInfo {
  pub name: &'static str,
  pub description: &'static str,
  pub args: Option<Vec<Arg>>,
  pub flags: Option<Vec<Flag>>,
}

impl CommandInfo {
  pub fn new(name: &'static str, description: &'static str) -> Self {
    CommandInfo {
      name,
      description,
      args: None,
      flags: None,
    }
  }

  pub fn args(mut self, args: Vec<Arg>) -> Self {
    self.args = Some(args);
    self
  }

  pub fn flags(mut self, flags: Vec<Flag>) -> Self {
    self.flags = Some(flags);
    self
  }

  pub fn with_help(self) -> Self {
    let mut flags = match self.flags {
      Some(flags) => flags,
      None => vec![],
    };

    flags.push(Flag::with_names("help", "h").description("Print help text"));

    CommandInfo {
      flags: Some(flags),
      ..self
    }
  }
}

pub struct Arg {
  pub name: &'static str,
  pub description: &'static str,
  pub default: Option<&'static str>,
  pub required: bool,
}

impl Arg {
  pub fn new(name: &'static str, description: &'static str) -> Self {
    Arg {
      name,
      description,
      default: None,
      required: false,
    }
  }

  pub fn default(mut self, default: &'static str) -> Self {
    self.default = Some(default);
    self
  }
}

pub struct Flag {
  pub long_name: &'static str,
  pub short_name: Option<&'static str>,
  pub style: FlagStyle,
  pub description: &'static str,
  pub value_name: Option<&'static str>,
  pub default: Option<&'static str>,
  pub possible_values: Option<Vec<&'static str>>,
}

impl Flag {
  pub fn with_names(long_name: &'static str, short_name: &'static str) -> Self {
    Flag {
      long_name,
      short_name: Some(short_name),
      description: "",
      value_name: None,
      default: None,
      possible_values: None,
      style: FlagStyle::Boolean,
    }
  }

  pub fn description(mut self, description: &'static str) -> Self {
    self.description = description;
    self
  }

  pub fn value_name(mut self, value_name: &'static str) -> Self {
    self.value_name = Some(value_name);
    self
  }

  pub fn default(mut self, default: &'static str) -> Self {
    self.default = Some(default);
    self
  }

  pub fn possible_values(mut self, possible_values: Vec<&'static str>) -> Self {
    self.possible_values = Some(possible_values);
    self
  }

  pub fn single_value(mut self) -> Self {
    self.style = FlagStyle::SingleValue;
    self
  }

  #[allow(dead_code)]
  pub fn multiple_values(mut self) -> Self {
    self.style = FlagStyle::MultipleValues;
    self
  }

  pub fn supports_value(&self, value: &String) -> bool {
    match &self.possible_values {
      None => true,

      Some(values) => {
        for val in values {
          if *val == value {
            return true;
          }
        }

        false
      }
    }
  }
}

#[derive(PartialEq)]
pub enum FlagStyle {
  Boolean,
  SingleValue,
  MultipleValues,
}

impl fmt::Display for CommandInfo {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    writeln!(
      f,
      "{} {} - version {}\n",
      colors::bold(BINARY_NAME),
      colors::bold(self.name),
      VERSION,
    )?;
    writeln!(f, "{}\n", self.description)?;

    writeln!(f, "{}", colors::bold("Usage:"))?;
    write!(f, "  {} {}", BINARY_NAME, self.name)?;

    let mut max_arg_length = 0;
    if let Some(args) = &self.args {
      for arg in args {
        max_arg_length = std::cmp::max(max_arg_length, arg.name.len() + 2);

        if arg.required {
          write!(f, " <{}>", arg.name)?;
        } else {
          write!(f, " [<{}>]", arg.name)?;
        }
      }
    }

    if self.flags.is_some() {
      write!(f, " [options]")?;
    }

    if let Some(args) = &self.args {
      write!(f, "\n\n{}", colors::bold("Arguments:"))?;

      for arg in args {
        write!(
          f,
          "\n  {:width$}   {}",
          format!("<{}>", arg.name),
          arg.description,
          width = max_arg_length
        )?;

        if let Some(default) = arg.default {
          write!(f, " (default: {})", default)?;
        }
      }
    }

    if let Some(flags) = &self.flags {
      let mut max_flag_length = 0;

      for flag in flags {
        max_flag_length = std::cmp::max(max_flag_length, flag.long_name.len());
      }

      write!(f, "\n\n{}", colors::bold("Options:"))?;

      for flag in flags {
        write!(f, "\n  ")?;

        if let Some(name) = flag.short_name {
          write!(f, "-{}, ", name)?;
        }

        write!(
          f,
          "--{:width$}   {}",
          flag.long_name,
          flag.description,
          width = max_flag_length
        )?;

        if let Some(values) = &flag.possible_values {
          write!(f, " (one of: {})", values.join(", "))?;
        }

        if let Some(default) = &flag.default {
          write!(f, " (default: {})", default)?;
        }
      }
    }

    Ok(())
  }
}
