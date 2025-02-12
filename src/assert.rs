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
    file_name: &str,
    golden: &str,
    output: &str,
    errs: &mut Vec<AssertError>,
  );
}

#[derive(Debug, thiserror::Error)]
pub enum AssertError {
  #[error("execute {0:?}: {1}")]
  Executes(Vec<String>, io::Error),
  #[error("exit code, want: {want}, got: {got}")]
  ExitCode { want: i32, got: i32 },
  #[error("file \"{0}\": {1}")]
  UnableToRead(String, io::Error),
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
}

pub(crate) struct DisplayErrs<'a, E: fmt::Display>(pub(crate) &'a Vec<E>);
impl<E: fmt::Display> fmt::Display for DisplayErrs<'_, E> {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for err in self.0 {
      writeln!(f, "ERROR {err}")?;
    }
    Ok(())
  }
}

impl Assert {
  #[inline]
  async fn save_output(
    name: String,
    work_dir: PathBuf,
    output: Arc<Output>,
  ) -> [Option<AssertError>; 2] {
    let stdout = work_dir.join(format!("{name}.stdout"));
    let stderr = work_dir.join(format!("{name}.stderr"));
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
    name: String,
    work_dir: PathBuf,
    golden_dir: PathBuf,
    output: Arc<Output>,
  ) -> Vec<AssertError> {
    let mut errs = Vec::new();
    // write stderr/stdout for debug
    let write_future = {
      let name = name.clone();
      let work_dir = work_dir.clone();
      let output = output.clone();
      tokio::spawn(async move { Self::save_output(name, work_dir, output).await })
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
          let work_dir = work_dir.clone();
          let golden_dir = golden_dir.clone();
          let output = output.clone();
          tokio::spawn(async move {
            golden.process_assert(name, work_dir, golden_dir, output).await
          })
        })
        .collect()
    } else {
      Vec::new()
    };

    // await
    for f in futures.into_iter() {
      errs.extend(f.await.expect("join handle"));
    }
    match write_future.await.expect("join handle") {
      [None, None] => {}
      [None, Some(e)] => errs.push(e),
      [Some(e), None] => errs.push(e),
      [Some(e1), Some(e2)] => errs.extend([e1, e2]),
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
  fn validate(&self) -> Result<(), impl Display> {
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
  value: f64,
  epsilon: Option<f64>,
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
    name: String,
    work_dir: PathBuf,
    golden_dir: PathBuf,
    output: Arc<Output>,
  ) -> Vec<AssertError> {
    fn read(path: impl AsRef<Path>) -> Result<String, AssertError> {
      read_to_string(&path)
        .map_err(|e| AssertError::UnableToRead(path.as_ref().display().to_string(), e))
    }
    let mut errs = Vec::new();
    let stdout_name = format!("{name}.stdout");
    let stderr_name = format!("{name}.stderr");
    if self.file == stdout_name {
      match (read(golden_dir.join(&stdout_name)), core::str::from_utf8(&output.stdout)) {
        (Ok(golden), Ok(output)) => self.assert(&stdout_name, &golden, output, &mut errs),
        (Ok(_), Err(_)) => errs.push(AssertError::Stdout),
        (Err(e), Ok(_)) => errs.push(e),
        (Err(e), Err(_)) => errs.extend([e, AssertError::Stdout]),
      }
    } else if self.file == stderr_name {
      match (read(golden_dir.join(&stderr_name)), core::str::from_utf8(&output.stderr)) {
        (Ok(golden), Ok(output)) => self.assert(&stderr_name, &golden, output, &mut errs),
        (Ok(_), Err(_)) => errs.push(AssertError::Stderr),
        (Err(e), Ok(_)) => errs.push(e),
        (Err(e), Err(_)) => errs.extend([e, AssertError::Stderr]),
      }
    } else {
      match (read(golden_dir.join(&self.file)), read(work_dir.join(&self.file))) {
        (Ok(golden), Ok(output)) => self.assert(&self.file, &golden, &output, &mut errs),
        (Ok(_), Err(e)) => errs.push(e),
        (Err(e), Ok(_)) => errs.push(e),
        (Err(e1), Err(e2)) => errs.extend([e1, e2]),
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
            if emphasized {
              write!(f, "{}", value)?;
            } else {
              write!(f, "{}", value)?;
            }
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
    file_name: &str,
    golden: &str,
    output: &str,
    errs: &mut Vec<AssertError>,
  ) {
    if let Some(true) = self.equal {
      if output != golden {
        errs.push(AssertError::Eq {
          file_name: file_name.to_owned(),
          diffs: TextDiffs(golden.to_owned(), output.to_owned()),
        });
      }
    }
    if let Some(vec) = &self.r#match {
      for m in vec {
        m.assert(file_name, golden, output, errs);
      }
    }
    if let Some(vec) = &self.value {
      for v in vec {
        v.assert(file_name, golden, output, errs);
      }
    }
  }
}

impl AssertT for Value {
  fn assert(
    &self,
    file_name: &str,
    golden: &str,
    output: &str,
    errs: &mut Vec<AssertError>,
  ) {
    todo!()
  }
}

#[derive(Debug)]
enum MatchCond {
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

impl fmt::Display for MatchReport {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(
      f,
      "pattern '{}' want {}{}, got: {}",
      self.pattern,
      match self.cond {
        Some(MatchCond::AtLeast) => "at least ",
        Some(MatchCond::AtMost) => "at most ",
        None => "",
      },
      self.count,
      self.matches.len()
    )?;
    for (idx, (line, res)) in self.matches.iter().enumerate() {
      writeln!(f, "  #{idx} at line {line}: {res:?}")?;
    }
    Ok(())
  }
}

impl AssertT for Match {
  fn assert(&self, file_name: &str, _: &str, output: &str, errs: &mut Vec<AssertError>) {
    let mut last_bgn = 0;
    let mut last_line = 0;
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
