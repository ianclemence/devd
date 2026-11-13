//! How a team run reads.
//!
//! A run is a set of tasks, each in a state. Three things are asked of it everywhere devd shows it (the window, the
//! assistant, a script): is this task finished, how far along is the run, and what is happening right now. They are
//! answered here, once, so that the window and the assistant never say different things about the same run.

/// One task, as far as reading a run goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Its key in the plan, which what depends on it names.
    pub key: String,
    /// The role that owns it.
    pub role: String,
    /// Where it stands: `pending`, `running`, `blocked`, `done`…
    pub state: String,
    /// The keys of what it waits for.
    pub depends_on: Vec<String>,
    /// Seconds since it last did anything, when it is working.
    pub quiet_secs: Option<u64>,
}

/// Whether a state means the task is finished and its work stands.
pub fn is_done(state: &str) -> bool {
    matches!(state, "done" | "verified" | "accepted" | "skipped")
}

/// Whether a state means the task has stopped without finishing.
pub fn is_stuck(state: &str) -> bool {
    matches!(
        state,
        "blocked" | "failed" | "cancelled" | "unverified" | "waiting_approval"
    )
}

/// How far along a run is: the tasks finished, and the tasks there are. The planner is not counted, because its job is
/// the plan and the count is of the work the plan holds; a task accepted by hand is finished. This is the rule the
/// server's own list of runs uses, so a run reads the same wherever it is shown.
pub fn progress(tasks: &[Task]) -> (usize, usize) {
    let work = tasks.iter().filter(|t| t.role != "planner");
    let total = work.clone().count();
    let done = work
        .filter(|t| matches!(t.state.as_str(), "done" | "skipped"))
        .count();
    (done, total)
}

/// A span in the plainest words: "12 seconds", "3 minutes", "2 hours".
pub fn span(secs: u64) -> String {
    let (n, unit) = match secs {
        0..=89 => (secs, "second"),
        90..=5399 => ((secs + 30) / 60, "minute"),
        _ => ((secs + 1800) / 3600, "hour"),
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// How long without a sign of life makes a working task worth a second look.
pub const QUIET_AFTER: u64 = 5 * 60;

/// What is happening in a run, in a sentence a person can read without opening anything: who is working and since when,
/// what is waiting for whom, and what needs the person. The answer to "how is it going?", so nobody has to ask for it.
///
/// `run_state` is the run's own: a paused run says so before anything else.
pub fn status_line(run_state: &str, tasks: &[Task]) -> String {
    let work: Vec<&Task> = tasks.iter().filter(|t| t.role != "planner").collect();
    if work.is_empty() {
        return match run_state {
            "planning" => "The planner is reading the goal and writing the plan.".to_string(),
            _ => "No tasks yet.".to_string(),
        };
    }
    let running: Vec<&&Task> = work.iter().filter(|t| t.state == "running").collect();
    let stuck: Vec<&&Task> = work.iter().filter(|t| is_stuck(&t.state)).collect();
    let (done, total) = progress(tasks);
    if done == total && total > 0 {
        return format!("All {total} tasks are finished.");
    }
    let mut parts: Vec<String> = Vec::new();
    if run_state == "paused" {
        parts.push("The run is paused".to_string());
    }
    if !stuck.is_empty() {
        let who = stuck
            .iter()
            .map(|t| format!("{} is {}", t.key, t.state.replace('_', " ")))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("Needs you: {who}"));
    }
    if !running.is_empty() {
        let who = running
            .iter()
            .map(|t| match t.quiet_secs {
                Some(s) if s >= QUIET_AFTER => format!(
                    "{} ({}) has not done anything for {}",
                    t.key,
                    t.role,
                    span(s)
                ),
                Some(s) if s >= 20 => format!("{} ({}) last active {} ago", t.key, t.role, span(s)),
                _ => format!("{} ({}) is working", t.key, t.role),
            })
            .collect::<Vec<_>>()
            .join("; ");
        parts.push(who);
    }
    // What waits, and for what.
    let waiting: Vec<&&Task> = work.iter().filter(|t| t.state == "pending").collect();
    if !waiting.is_empty() && running.is_empty() && stuck.is_empty() && run_state != "paused" {
        parts.push(format!(
            "Nothing is running; {} task{} waiting to start",
            waiting.len(),
            if waiting.len() == 1 { " is" } else { "s are" }
        ));
    } else if !waiting.is_empty() {
        let next: Vec<String> = waiting
            .iter()
            .filter(|t| {
                t.depends_on
                    .iter()
                    .all(|d| tasks.iter().any(|x| &x.key == d && is_done(&x.state)))
            })
            .map(|t| t.key.clone())
            .collect();
        if !next.is_empty() {
            parts.push(format!("Ready to start: {}", next.join(", ")));
        }
        let held = waiting.len() - next.len();
        if held > 0 {
            parts.push(format!("{held} waiting on earlier work"));
        }
    }
    format!("{}.", parts.join(". "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(key: &str, role: &str, state: &str, deps: &[&str]) -> Task {
        Task {
            key: key.into(),
            role: role.into(),
            state: state.into(),
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            quiet_secs: None,
        }
    }

    #[test]
    fn progress_counts_the_work_not_the_plan_and_counts_a_task_accepted_by_hand() {
        let tasks = vec![
            t("plan", "planner", "done", &[]),
            t("design", "architect", "skipped", &[]),
            t("store", "backend", "done", &["design"]),
            t("cron", "implementer", "running", &["design"]),
            t("api", "backend", "pending", &["store"]),
        ];
        assert_eq!(
            progress(&tasks),
            (2, 4),
            "two of the four tasks of work are finished, the same as the server's list says"
        );
        assert_eq!(progress(&[]), (0, 0));
    }

    #[test]
    fn the_status_line_says_who_is_working_and_since_when_what_is_ready_and_what_needs_the_person()
    {
        let mut tasks = vec![
            t("plan", "planner", "done", &[]),
            t("design", "architect", "skipped", &[]),
            t("cron", "implementer", "running", &["design"]),
            t("store", "backend", "done", &["design"]),
            t("sched", "implementer", "pending", &["cron", "store"]),
            t("api", "backend", "pending", &["store"]),
        ];
        tasks[2].quiet_secs = Some(12);
        let line = status_line("running", &tasks);
        assert!(line.contains("cron (implementer) is working"), "{line}");
        assert!(line.contains("Ready to start: api"), "{line}");
        assert!(line.contains("1 waiting on earlier work"), "{line}");
        tasks[2].quiet_secs = Some(7 * 60);
        assert!(status_line("running", &tasks).contains("has not done anything for 7 minutes"));
        tasks[2].state = "blocked".into();
        let line = status_line("paused", &tasks);
        assert!(line.starts_with("The run is paused"), "{line}");
        assert!(line.contains("Needs you: cron is blocked"), "{line}");
    }

    #[test]
    fn a_finished_run_says_so_and_a_new_plan_says_the_planner_is_at_work() {
        let done = vec![
            t("plan", "planner", "done", &[]),
            t("a", "qa", "done", &[]),
            t("b", "docs", "skipped", &[]),
        ];
        assert_eq!(status_line("done", &done), "All 2 tasks are finished.");
        assert_eq!(
            status_line("planning", &[]),
            "The planner is reading the goal and writing the plan."
        );
    }

    #[test]
    fn spans_read_as_a_person_would_say_them() {
        assert_eq!(span(1), "1 second");
        assert_eq!(span(45), "45 seconds");
        assert_eq!(span(120), "2 minutes");
        assert_eq!(span(7200), "2 hours");
    }
}
