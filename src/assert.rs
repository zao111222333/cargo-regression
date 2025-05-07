use core::fmt;
use std::{
  fmt::Display,
  fs::read_to_string,
  io,
  ops::Deref,
  path::{Path, PathBuf},
  process::Output,
  sync::Arc,
};

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "kebab-case")]
pub struct Assert {
  pub exit_code: Option<i32>,
  pub golden: Option<Vec<Golden>>,
}

trait AssertT {
  fn assert(
    &self,
    config: AssertConfig,
    file_name: &str,
    golden: Option<&str>,
    output: &str,
    errs: &mut Vec<AssertError>,
  );
}

#[derive(Debug, thiserror::Error)]
pub enum AssertError {
  #[error("{0}: {1}")]
  ProcessExec(String, io::Error),
  #[error("execute {0:?}: {1}")]
  Executes(Vec<String>, io::Error),
  #[error("exit code, want: {want}, got: {got}")]
  ExitCode { want: i32, got: i32 },
  #[error("file \"{0}\": Unable to read")]
  UnableToRead(String),
  #[error("dir \"{0}\": {1}")]
  UnableToReadDir(String, io::Error),
  #[error("dir \"{0}\": {1}")]
  UnableToCreateDir(String, io::Error),
  #[error("dir \"{0}\": {1}")]
  UnableToDeleteDir(String, io::Error),
  #[error("link \"{0}\" to \"{1}\": {2}")]
  LinkFile(String, String, io::Error),
  #[error("file \"{file_name}\" not equal\n{diffs}")]
  Eq { file_name: String, diffs: TextDiffs },
  #[error("write file \"{0}\": {1}")]
  Write(String, io::Error),
  #[error("unable to encode stdout to utf8")]
  Stdout,
  #[error("unable to encode stderr to utf8")]
  Stderr,
  #[error("terminated by a signal")]
  Terminated,
  #[error(
    "You should specify one and only one of `count`, `count-at-least`, `count-at-most`"
  )]
  CountConfig,
  #[error("file \"{0}\" match failed\n{1}")]
  Match(String, MatchReport),
  #[error("file \"{0}\" assert value failed\n{1}")]
  Value(String, ValueReport),
  #[error("regular expression: {0}")]
  Regex(regex::Error),
}

pub(crate) struct DisplayErrs<'a, E: fmt::Display>(pub(crate) &'a Vec<E>);
impl<E: fmt::Display> fmt::Display for DisplayErrs<'_, E> {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for err in self.0 {
      writeln!(f, "{err}")?;
    }
    Ok(())
  }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AssertConfig {
  pub(crate) epsilon: f32,
}
impl Assert {
  #[inline]
  async fn save_output(
    name: &str,
    workdir: &Path,
    output: &Output,
  ) -> [Option<AssertError>; 2] {
    let stdout = workdir.join(format!("{name}.stdout"));
    let stderr = workdir.join(format!("{name}.stderr"));
    [
      // save stdout to {{name}}.stdout
      if let Err(e) = tokio::fs::write(&stdout, &output.stdout).await {
        Some(AssertError::Write(stdout.display().to_string(), e))
      } else {
        None
      },
      // save stderr to {{name}}.stderr
      if let Err(e) = tokio::fs::write(&stderr, &output.stderr).await {
        Some(AssertError::Write(stderr.display().to_string(), e))
      } else {
        None
      },
    ]
  }
  #[inline]
  pub async fn assert(
    self,
    config: AssertConfig,
    name: String,
    workdir: PathBuf,
    golden_dir: PathBuf,
    output: Arc<Output>,
  ) -> Vec<AssertError> {
    let mut errs = match Self::save_output(&name, &workdir, &output).await {
      [None, None] => Vec::new(),
      [Some(e), None] | [None, Some(e)] => vec![e],
      [Some(e1), Some(e2)] => vec![e1, e2],
    };
    // exit_code
    let exit_code_want = self.exit_code.unwrap_or(0);
    if let Some(exit_code_got) = output.status.code() {
      if exit_code_want != exit_code_got {
        errs.push(AssertError::ExitCode { want: exit_code_want, got: exit_code_got });
      }
    } else {
      errs.push(AssertError::Terminated);
    }
    // golden
    let futures = if let Some(goldens) = self.golden {
      goldens
        .into_iter()
        .map(|golden| {
          let name = name.clone();
          let workdir = workdir.clone();
          let golden_dir = golden_dir.clone();
          let output = output.clone();
          tokio::spawn(golden.process_assert(config, name, workdir, golden_dir, output))
        })
        .collect()
    } else {
      Vec::new()
    };
    // await
    for f in futures.into_iter() {
      errs.extend(f.await.expect("join handle"));
    }
    errs
  }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Golden {
  pub file: String,
  equal: Option<bool>,
  r#match: Option<Vec<Match>>,
  value: Option<Vec<Value>>,
}

impl Golden {
  fn _validate(&self) -> Result<(), impl Display> {
    if self.equal.is_none() && self.r#match.is_none() && self.value.is_none() {
      return Err(format!("no assert for file \"{}\"", self.file));
    }
    Ok(())
  }
}

#[derive(Debug, Clone)]
struct PatternMatch(regex::Regex);
impl Deref for PatternMatch {
  type Target = regex::Regex;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl<'de> Deserialize<'de> for PatternMatch {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let s = String::deserialize(deserializer)?;
    match regex::Regex::new(&s) {
      Ok(reg) => Ok(PatternMatch(reg)),
      Err(e) => Err(serde::de::Error::custom(e)),
    }
  }
}
impl Serialize for PatternMatch {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: serde::Serializer,
  {
    serializer.serialize_str(self.as_str())
  }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Value {
  pattern_before: Option<PatternMatch>,
  pattern_after: Option<PatternMatch>,
  value: Option<f32>,
  value_at_most: Option<f32>,
  value_at_least: Option<f32>,
  epsilon: Option<f32>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Match {
  pattern: PatternMatch,
  count: Option<usize>,
  count_at_most: Option<usize>,
  count_at_least: Option<usize>,
}
impl Golden {
  #[inline]
  async fn process_assert(
    self,
    config: AssertConfig,
    name: String,
    workdir: PathBuf,
    golden_dir: PathBuf,
    output: Arc<Output>,
  ) -> Vec<AssertError> {
    fn read(path: impl AsRef<Path>) -> Option<String> {
      read_to_string(&path).ok()
    }
    let mut errs = Vec::new();
    let stdout_name = format!("{name}.stdout");
    let stderr_name = format!("{name}.stderr");
    let golden = read(golden_dir.join(&self.file));
    let golden_str = golden.as_deref();
    if self.file == stdout_name {
      match core::str::from_utf8(&output.stdout) {
        Ok(output) => self.assert(config, &stdout_name, golden_str, output, &mut errs),
        Err(_) => errs.push(AssertError::Stdout),
      }
    } else if self.file == stderr_name {
      match core::str::from_utf8(&output.stderr) {
        Ok(output) => self.assert(config, &stderr_name, golden_str, output, &mut errs),
        Err(_) => errs.push(AssertError::Stderr),
      }
    } else {
      match read(workdir.join(&self.file)) {
        Some(output) => self.assert(config, &stderr_name, golden_str, &output, &mut errs),
        None => errs.push(AssertError::UnableToRead(self.file)),
      }
    }
    errs
  }
}

#[derive(Debug)]
pub(crate) struct TextDiffs(String, String);
// https://github.com/mitsuhiko/similar/blob/main/examples/terminal-inline.rs
impl fmt::Display for TextDiffs {
  #[inline]
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    use similar::ChangeTag;
    struct Line(Option<usize>);
    impl fmt::Display for Line {
      fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
          None => write!(f, "    "),
          Some(idx) => write!(f, "{:<4}", idx + 1),
        }
      }
    }
    let diff = similar::TextDiff::from_lines(&self.0, &self.1);
    for (idx, group) in diff.grouped_ops(3).iter().enumerate() {
      if idx > 0 {
        writeln!(f, "{:-^1$}", "-", 80)?;
      }
      writeln!(f, "old new")?;
      for op in group {
        for change in diff.iter_inline_changes(op) {
          let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
          };
          write!(
            f,
            "{}{} |{}",
            Line(change.old_index()),
            Line(change.new_index()),
            sign,
          )?;
          for (emphasized, value) in change.iter_strings_lossy() {
            _ = emphasized;
            write!(f, "{}", value)?;
          }
          if change.missing_newline() {
            writeln!(f)?;
          }
        }
      }
    }
    Ok(())
  }
}

impl AssertT for Golden {
  fn assert(
    &self,
    config: AssertConfig,
    file_name: &str,
    golden: Option<&str>,
    output: &str,
    errs: &mut Vec<AssertError>,
  ) {
    if let Some(true) = self.equal {
      if let Some(golden) = golden {
        if output != golden {
          errs.push(AssertError::Eq {
            file_name: file_name.to_owned(),
            diffs: TextDiffs(golden.to_owned(), output.to_owned()),
          });
        }
      } else {
        errs.push(AssertError::UnableToRead(file_name.into()))
      }
    }
    if let Some(vec) = &self.r#match {
      for m in vec {
        m.assert(config, file_name, golden, output, errs);
      }
    }
    if let Some(vec) = &self.value {
      for v in vec {
        v.assert(config, file_name, golden, output, errs);
      }
    }
  }
}

#[derive(Debug)]
pub enum ValueReport {
  Config,
  NegativeEpsilon(f32),
  AssertFail {
    line: usize,
    pattern: regex::Regex,
    matched: String,
    want_value: f32,
    got_value: f32,
    epsilon: f32,
    cond: Option<MatchCond>,
  },
  NoMatch {
    pattern: regex::Regex,
  },
  ParseFloat {
    line: usize,
    pattern: regex::Regex,
    matched: String,
  },
}

impl fmt::Display for ValueReport {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      ValueReport::Config => write!(
        f,
        "You should specify one and only one of `value`, `value-at-least`, `value-at-most`"
      ),
      ValueReport::AssertFail {
        line,
        pattern,
        matched,
        want_value,
        got_value,
        epsilon,
        cond,
      } => {
        let (msg1, msg2) = match cond {
          Some(MatchCond::AtLeast) => ("> ", "-"),
          Some(MatchCond::AtMost) => ("< ", "+"),
          None => ("", "±"),
        };
        write!(
          f,
          "pattern '{pattern}' caputred '{matched}' at line {line}, want {msg1}{want_value}{msg2}{epsilon}, got: {got_value}"
        )
      }
      ValueReport::NoMatch { pattern } => write!(f, "can not match pattern '{pattern}'"),
      ValueReport::ParseFloat { line, pattern, matched } => {
        write!(f, "pattern '{pattern}' caputred '{matched}' at line {line}, parse failed")
      }
      ValueReport::NegativeEpsilon(epsilon) => {
        write!(f, "the epsilon = {epsilon} is negative")
      }
    }
  }
}

impl AssertT for Value {
  fn assert(
    &self,
    config: AssertConfig,
    file_name: &str,
    _: Option<&str>,
    output: &str,
    errs: &mut Vec<AssertError>,
  ) {
    let (want_value, cond) = match (self.value, self.value_at_least, self.value_at_most) {
      (None, None, Some(value)) => (value, Some(MatchCond::AtMost)),
      (None, Some(value), None) => (value, Some(MatchCond::AtLeast)),
      (Some(value), None, None) => (value, None),
      _ => {
        errs.push(AssertError::Value(file_name.into(), ValueReport::Config));
        return;
      }
    };
    let re = match match (&self.pattern_before, &self.pattern_after) {
      (None, None) => {
        Err(regex::Error::Syntax("Empty `pattern-before` and `pattern-after`".into()))
      }
      (None, Some(after)) => regex::Regex::new(&format!(
        r"([-+]?\d*\.?\d+(?:[eE][-+]?\d+)?)\s*{}",
        after.as_str()
      )),
      (Some(before), None) => regex::Regex::new(&format!(
        r"{}\s*([-+]?\d*\.?\d+(?:[eE][-+]?\d+)?)",
        before.as_str()
      )),
      (Some(before), Some(after)) => regex::Regex::new(&format!(
        r"{}\s*([-+]?\d*\.?\d+(?:[eE][-+]?\d+)?)\s*{}",
        before.as_str(),
        after.as_str()
      )),
    } {
      Ok(re) => re,
      Err(e) => {
        errs.push(AssertError::Regex(e));
        return;
      }
    };
    let epsilon = self.epsilon.unwrap_or(config.epsilon);
    if epsilon.is_sign_negative() {
      errs.push(AssertError::Value(
        file_name.into(),
        ValueReport::NegativeEpsilon(epsilon),
      ));
      return;
    }
    let mut last_bgn = 0;
    let mut line = 1;
    let mut captured = false;
    for cap in re.captures_iter(output) {
      captured = true;
      let overall_mat = cap.get(0).unwrap();
      let capture_mat = cap.get(1).unwrap();
      let bgn = overall_mat.start();
      line += output[last_bgn..bgn].matches('\n').count();
      last_bgn = bgn;
      match capture_mat.as_str().parse::<f32>() {
        Ok(got_value) => {
          if match cond {
            Some(MatchCond::AtLeast) => got_value + epsilon < want_value,
            Some(MatchCond::AtMost) => got_value > want_value + epsilon,
            None => got_value > want_value + epsilon || got_value < want_value - epsilon,
          } {
            errs.push(AssertError::Value(
              file_name.to_owned(),
              ValueReport::AssertFail {
                line,
                pattern: re.clone(),
                matched: overall_mat.as_str().into(),
                want_value,
                got_value,
                epsilon,
                cond,
              },
            ));
          }
        }
        Err(_) => {
          errs.push(AssertError::Value(
            file_name.to_owned(),
            ValueReport::ParseFloat {
              line,
              pattern: re.clone(),
              matched: overall_mat.as_str().into(),
            },
          ));
        }
      }
    }
    if !captured {
      errs.push(AssertError::Value(
        file_name.to_owned(),
        ValueReport::NoMatch { pattern: re },
      ));
    }
  }
}

#[derive(Debug, Clone, Copy)]
pub enum MatchCond {
  AtMost,
  AtLeast,
}

#[derive(Debug)]
pub struct MatchReport {
  pattern: regex::Regex,
  count: usize,
  cond: Option<MatchCond>,
  matches: Vec<(usize, String)>,
}

fn cond_str(cond: Option<MatchCond>) -> &'static str {
  match cond {
    Some(MatchCond::AtLeast) => "at least ",
    Some(MatchCond::AtMost) => "at most ",
    None => "",
  }
}
impl fmt::Display for MatchReport {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(
      f,
      "pattern '{}' want {}{}, got: {}",
      self.pattern,
      cond_str(self.cond),
      self.count,
      self.matches.len()
    )?;
    for (idx, (line, res)) in self.matches.iter().enumerate() {
      writeln!(f, "  #{} at line {line}: {res:?}", idx + 1)?;
    }
    Ok(())
  }
}

impl AssertT for Match {
  fn assert(
    &self,
    _: AssertConfig,
    file_name: &str,
    _: Option<&str>,
    output: &str,
    errs: &mut Vec<AssertError>,
  ) {
    let mut last_bgn = 0;
    let mut last_line = 1;
    let matches: Vec<(usize, String)> = self
      .pattern
      .find_iter(output)
      .map(|mat| {
        let bgn = mat.start();
        last_line += output[last_bgn..bgn].matches('\n').count();
        last_bgn = bgn;
        (last_line, mat.as_str().to_owned())
      })
      .collect();
    let (count, cond) = match (self.count, self.count_at_most, self.count_at_least) {
      (Some(count), None, None) => {
        if count != matches.len() {
          (count, None)
        } else {
          return;
        }
      }
      (None, Some(count), None) => {
        if count < matches.len() {
          (count, Some(MatchCond::AtMost))
        } else {
          return;
        }
      }
      (None, None, Some(count)) => {
        if count > matches.len() {
          (count, Some(MatchCond::AtLeast))
        } else {
          return;
        }
      }
      _ => {
        errs.push(AssertError::CountConfig);
        return;
      }
    };
    errs.push(AssertError::Match(
      file_name.to_owned(),
      MatchReport {
        pattern: self.pattern.0.clone(),
        count,
        cond,
        matches,
      },
    ));
  }
}

#[test]
fn valuematch() {
  let re = regex::Regex::new(&format!(
    r"{}\s*([-+]?\d*\.?\d+(?:[eE][-+]?\d+)?)\s*{}",
    "Values", "aaa"
  ))
  .unwrap();
  let cap = re.captures("Values -3.14e-2 aaa").unwrap();
  dbg!(cap.get(0));
  dbg!(cap.get(1));
}
