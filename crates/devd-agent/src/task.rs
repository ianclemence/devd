//! A task document, and the one line a run of it reports.
//!
//! `devd run --task task.json` runs one prepared piece of work with the assistant and
//! exits, so a script or a CI job can use devd without a person at the keyboard. This is
//! the contract it keeps, as plain data and functions so every rule is a test:
//!
//! * **Strict input.** One JSON object, at most 1 MiB. An unknown field, a duplicate field,
//!   a wrong type, a second document after the first, or a version this build does not
//!   know is refused before anything runs. A typo never silently changes what runs.
//! * **One line out.** Standard output carries exactly one JSON line, [`Report`], however it
//!   ends. Everything else a run says goes to standard error.
//! * **An exit code that means something.** A CI job reads the code, not the prose.
//!   "Failed", "needs a person", "timed out" and "finished but not proven" are four
//!   different things to do next, so they are four different codes.
//! * **No retry.** A run that failed, timed out or ended uncertain is reported as it is.
//!   Running it again is a decision for whoever read the report: the first attempt may
//!   already have changed things.
//! * **No waiting on nobody.** If the agent needs a person's approval and nobody can give
//!   it, the run stops and says which approval, instead of sitting until its deadline.

use serde::{Deserialize, Serialize};

/// The version of the task document this build reads.
pub const VERSION: u32 = 1;

/// The largest task document, in bytes.
pub const MAX_BYTES: usize = 1 << 20;

/// The longest a task may be given to run: a day.
pub const MAX_DURATION_MS: u64 = 24 * 60 * 60 * 1000;

/// The prepared work.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Task {
    /// Must be [`VERSION`].
    pub version: u32,
    /// A name for the caller to correlate the report with. Not a claim or a deduplication key.
    pub task_id: String,
    /// What to do. Include the context and where to report: the agent starts with nothing else.
    pub prompt: String,
    /// The longest the run may take. The run is stopped, and reported as timed out, when it passes.
    pub max_duration_ms: u64,
    /// The project directory the agent works in.
    #[serde(default)]
    pub project: Option<String>,
    /// A provider id, for this run only.
    #[serde(default)]
    pub provider: Option<String>,
    /// A model, for this run only.
    #[serde(default)]
    pub model: Option<String>,
    /// `low`, `medium`, `high`, `extra` or `max`.
    #[serde(default)]
    pub effort: Option<String>,
    /// `auto`, `manual`, `plan` or `bypass`. Defaults to `auto`.
    #[serde(default)]
    pub permission: Option<String>,
}

/// Why a document was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Read and check a task document.
pub fn parse(bytes: &[u8]) -> Result<Task, Refused> {
    if bytes.len() > MAX_BYTES {
        return Err(Refused(format!("the task is {} bytes; the most is {MAX_BYTES}. Put the long part in a file and name it in the prompt", bytes.len())));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Refused("the task is not UTF-8 text".into()))?;
    // A derived reader refuses unknown and duplicate fields, wrong types and trailing text.
    let task: Task = serde_json::from_str(text).map_err(|e| Refused(format!("the task is not a valid version-1 document: {e}")))?;
    if task.version != VERSION {
        return Err(Refused(format!("the task is version {}; this devd reads version {VERSION}", task.version)));
    }
    if task.task_id.is_empty() || task.task_id.len() > 256 || task.task_id.chars().any(char::is_control) {
        return Err(Refused("`taskId` must be 1 to 256 bytes with no control characters".into()));
    }
    if task.prompt.trim().is_empty() {
        return Err(Refused("`prompt` is empty: say what to do".into()));
    }
    if task.max_duration_ms == 0 || task.max_duration_ms > MAX_DURATION_MS {
        return Err(Refused(format!("`maxDurationMs` must be between 1 and {MAX_DURATION_MS}")));
    }
    if let Some(e) = &task.effort {
        if !matches!(e.as_str(), "none" | "low" | "medium" | "high" | "extra" | "max") {
            return Err(Refused("`effort` must be low, medium, high, extra or max".into()));
        }
    }
    if let Some(p) = &task.permission {
        if !matches!(p.as_str(), "auto" | "manual" | "plan" | "bypass") {
            return Err(Refused("`permission` must be auto, manual, plan or bypass".into()));
        }
    }
    Ok(task)
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Finished, and the record proves it.
    Completed,
    /// Finished, but the record does not show what the run's policy asks for.
    Unproven,
    /// The agent stopped with an error.
    Failed,
    /// The document was refused before anything ran.
    Invalid,
    /// The agent is waiting for an approval nobody was there to give.
    NeedsPerson,
    /// The deadline passed. The run was stopped.
    TimedOut,
    /// Ended by a signal or by someone stopping it.
    Cancelled,
}

impl Status {
    /// The process exit code. Stable: scripts depend on it.
    pub fn exit_code(self) -> i32 {
        match self {
            Status::Completed => 0,
            Status::Failed => 1,
            Status::Invalid => 2,
            Status::NeedsPerson => 3,
            Status::TimedOut => 4,
            Status::Unproven => 5,
            Status::Cancelled => 130,
        }
    }
}

/// The one line a run writes to standard output.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The report's own version.
    pub version: u32,
    /// The task's `taskId`, when the document got far enough to have one.
    pub task_id: Option<String>,
    /// How it ended.
    pub status: Status,
    /// The exit code the process will return.
    pub exit_code: i32,
    /// The conversation the work happened in, to read or continue.
    pub thread: Option<String>,
    /// A `devd://` link to that conversation.
    pub link: Option<String>,
    /// How long it took.
    pub duration_ms: u64,
    /// The agent's last words: what it says it did.
    pub summary: String,
    /// What the run's policy still lacks, when `status` is `unproven`.
    pub missing: Vec<String>,
    /// Approval ids waiting for a person, when `status` is `needs_person`.
    pub pending_approvals: Vec<String>,
    /// Tokens read and written, and what they cost at list price.
    pub tokens: Tokens,
    /// What it cost, when the model's prices are known.
    pub cost_usd: Option<f64>,
    /// Why it did not complete, for a person.
    pub error: Option<String>,
}

/// Tokens used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tokens {
    /// Read by the model.
    pub input: u64,
    /// Written by the model.
    pub output: u64,
}

impl Report {
    /// A report for `status`, with nothing else filled in.
    pub fn of(status: Status, task_id: Option<String>) -> Self {
        Report {
            version: VERSION,
            task_id,
            status,
            exit_code: status.exit_code(),
            thread: None,
            link: None,
            duration_ms: 0,
            summary: String::new(),
            missing: Vec::new(),
            pending_approvals: Vec::new(),
            tokens: Tokens::default(),
            cost_usd: None,
            error: None,
        }
    }

    /// The line, with no newline inside it.
    pub fn line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"version\":1,\"status\":\"failed\",\"exitCode\":1}".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{"version":1,"taskId":"nightly-1","prompt":"Run the tests and report.","maxDurationMs":120000}"#;

    #[test]
    fn a_complete_document_is_read() {
        let t = parse(GOOD.as_bytes()).unwrap();
        assert_eq!((t.task_id.as_str(), t.max_duration_ms), ("nightly-1", 120_000));
        assert!(t.project.is_none() && t.permission.is_none());
        let full = r#"{"version":1,"taskId":"a","prompt":"p","maxDurationMs":1,"project":"D:\\x","effort":"high","permission":"plan"}"#;
        assert_eq!(parse(full.as_bytes()).unwrap().permission.as_deref(), Some("plan"));
    }

    #[test]
    fn a_typo_never_silently_changes_what_runs() {
        for (doc, why) in [
            (r#"{"version":1,"taskId":"a","prompt":"p","maxDurationMs":1,"promt":"x"}"#, "unknown field"),
            (r#"{"version":1,"taskId":"a","prompt":"p","prompt":"q","maxDurationMs":1}"#, "duplicate field"),
            (r#"{"version":1,"taskId":"a","prompt":"p","maxDurationMs":"soon"}"#, "wrong type"),
            (r#"{"version":1,"taskId":"a","prompt":"p","maxDurationMs":1} {"x":1}"#, "second document"),
            (r#"{"version":1,"taskId":"a","prompt":"p"}"#, "missing field"),
            ("not json", "malformed"),
            ("", "empty"),
        ] {
            assert!(parse(doc.as_bytes()).is_err(), "{why} was accepted: {doc}");
        }
    }

    #[test]
    fn values_are_checked_not_just_shapes() {
        let with = |patch: &str| format!(r#"{{"version":{patch}}}"#);
        assert!(parse(with(r#"2,"taskId":"a","prompt":"p","maxDurationMs":1"#).as_bytes()).unwrap_err().0.contains("version 2"));
        assert!(parse(with(r#"1,"taskId":"","prompt":"p","maxDurationMs":1"#).as_bytes()).is_err());
        assert!(parse(with(r#"1,"taskId":"a\nb","prompt":"p","maxDurationMs":1"#).as_bytes()).is_err());
        assert!(parse(with(r#"1,"taskId":"a","prompt":"  ","maxDurationMs":1"#).as_bytes()).is_err());
        assert!(parse(with(r#"1,"taskId":"a","prompt":"p","maxDurationMs":0"#).as_bytes()).is_err());
        assert!(parse(with(r#"1,"taskId":"a","prompt":"p","maxDurationMs":999999999999"#).as_bytes()).is_err());
        assert!(parse(with(r#"1,"taskId":"a","prompt":"p","maxDurationMs":1,"effort":"ludicrous""#).as_bytes()).is_err());
        assert!(parse(with(r#"1,"taskId":"a","prompt":"p","maxDurationMs":1,"permission":"yolo""#).as_bytes()).is_err());
    }

    #[test]
    fn a_document_over_the_limit_is_refused_before_it_is_parsed() {
        let big = format!(r#"{{"version":1,"taskId":"a","prompt":"{}","maxDurationMs":1}}"#, "x".repeat(MAX_BYTES));
        assert!(parse(big.as_bytes()).unwrap_err().0.contains("the most is"));
        assert!(parse(&[0xff, 0xfe, 0x00]).unwrap_err().0.contains("UTF-8"));
    }

    #[test]
    fn each_way_of_ending_has_its_own_exit_code() {
        let all = [Status::Completed, Status::Failed, Status::Invalid, Status::NeedsPerson, Status::TimedOut, Status::Unproven, Status::Cancelled];
        let mut codes: Vec<i32> = all.iter().map(|s| s.exit_code()).collect();
        assert_eq!(codes[0], 0, "only a proven completion is success");
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), all.len(), "two endings share a code, so a script cannot tell them apart");
    }

    #[test]
    fn the_report_is_one_line_of_json_a_script_can_read() {
        let mut r = Report::of(Status::NeedsPerson, Some("t-1".into()));
        r.summary = "line one\nline two".into();
        r.pending_approvals = vec!["ap1".into()];
        let line = r.line();
        assert!(!line.contains('\n'), "a newline inside the summary must be escaped, not break the line");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["status"], "needs_person");
        assert_eq!(v["exitCode"], 3);
        assert_eq!(v["taskId"], "t-1");
        assert_eq!(v["pendingApprovals"][0], "ap1");
        assert_eq!(v["version"], 1);
    }
}
