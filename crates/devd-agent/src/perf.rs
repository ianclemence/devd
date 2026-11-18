//! Numbers, not adjectives.
//!
//! "Fast" is a claim until it is measured, and a claim nobody re-checks is a story. This measures
//! the parts of devd that live in this process and that a person would feel: opening and migrating
//! the state database, writing conversations, listing them, appending the event log and reading a
//! conversation back. Each reading has a budget; a budget is a regression line, not a target, so it
//! is set several times above what the work costs on an ordinary machine.
//!
//! It is deliberately small. A benchmark that needs a dataset, a profile or a warm-up is a job for a
//! release, not a question an agent should be able to ask in one call.

use std::time::Instant;

use devd_core::error::Result;
use devd_core::state::Store;
use serde_json::{json, Value};

/// How many rows one reading uses. Enough for the shape of the work to show, small enough to run in
/// a second.
const ROWS: usize = 500;

/// One reading: what was measured, how long it took, and the line it must stay under.
struct Reading {
    name: &'static str,
    what: &'static str,
    ms: f64,
    budget_ms: f64,
}

impl Reading {
    fn ok(&self) -> bool {
        self.ms <= self.budget_ms
    }

    fn json(&self) -> Value {
        json!({
            "name": self.name,
            "what": self.what,
            "ms": (self.ms * 100.0).round() / 100.0,
            "budget_ms": self.budget_ms,
            "ok": self.ok(),
        })
    }
}

/// Measure this process's own work against its budgets.
pub fn measure() -> Result<Value> {
    let mut readings = Vec::new();

    let started = Instant::now();
    let store = Store::open_in_memory()?;
    store.migrate()?;
    readings.push(Reading {
        name: "database",
        what: "open the state database and apply every migration",
        ms: ms(started),
        budget_ms: 900.0,
    });

    let started = Instant::now();
    let mut thread = String::new();
    for n in 0..ROWS {
        let t = store.create_thread(&format!("conversation {n}"), None, None, None)?;
        if n == 0 {
            thread = t.id.clone();
        }
    }
    readings.push(Reading {
        name: "conversations written",
        what: "write 500 conversations with their first row",
        ms: ms(started),
        budget_ms: 800.0,
    });

    let started = Instant::now();
    let listed = store.list_threads(ROWS as i64)?;
    readings.push(Reading {
        name: "conversations listed",
        what: "list the 500 most recent conversations",
        ms: ms(started),
        budget_ms: 150.0,
    });
    debug_assert_eq!(listed.len(), ROWS);

    let started = Instant::now();
    for n in 0..ROWS {
        store.append_event(&thread, "user", &json!({ "text": format!("message {n}") }))?;
    }
    readings.push(Reading {
        name: "events appended",
        what: "append 500 events to one conversation's log",
        ms: ms(started),
        budget_ms: 250.0,
    });

    let started = Instant::now();
    let read = store.list_events(&thread, 0, 1000)?;
    readings.push(Reading {
        name: "conversation read",
        what: "read those 500 events back, as a window does",
        ms: ms(started),
        budget_ms: 150.0,
    });
    debug_assert_eq!(read.len(), ROWS);

    let ok = readings.iter().all(Reading::ok);
    Ok(json!({
        "ok": ok,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
        "measurements": readings.iter().map(Reading::json).collect::<Vec<_>>(),
        "note": "Budgets are regression lines, not targets: several times what the work costs on an ordinary machine. A miss is a change worth explaining, not a verdict on the machine.",
    }))
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_devd_measures_is_inside_its_budget() {
        let v = measure().expect("measuring does not fail");
        let rows = v["measurements"].as_array().expect("readings");
        assert_eq!(rows.len(), 5, "five readings: {v}");
        for row in rows {
            let name = row["name"].as_str().unwrap_or_default();
            let ms = row["ms"].as_f64().expect("a number");
            let budget = row["budget_ms"].as_f64().expect("a budget");
            // The suite runs hundreds of tests in parallel, so this is the "several times slower"
            // line, not the budget itself: a reading that misses its own budget is reported by the
            // capability and read by a person, and only a real regression fails the build.
            assert!(
                ms < budget * 4.0,
                "{name} took {ms}ms against a {budget}ms budget"
            );
        }
    }
}
