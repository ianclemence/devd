//! Numbers, not adjectives.
//!
//! "Fast" is a claim until it is measured, and a claim nobody re-checks is a story. This measures
//! the parts of devd that live in this process and that a person would feel: opening and migrating
//! the state database, writing conversations, listing them, appending the event log and reading a
//! conversation back. Each reading has a budget; a budget is a regression line, not a target, so it
//! is set several times above what the work costs on an ordinary machine.
//!
//! Every reading is the **best of three runs**. A microbenchmark reports the least-interfered run,
//! because this is measured from a window that is also drawing, from a server that is also serving
//! and from a test binary running hundreds of tests at once. Averaging would report the machine's
//! mood; the minimum reports the work.
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

/// How many times the work is run, keeping the fastest. Three is enough to see past one scheduling
/// hiccup without making a diagnostics page feel slow.
const RUNS: usize = 3;

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

/// Measure this process's own work against its budgets, and report the last window launch if one
/// was recorded beside the state.
pub fn measure(home: &std::path::Path) -> Result<Value> {
    let mut best = [f64::MAX; 5];
    for _ in 0..RUNS {
        for (at, ms) in run_once()?.into_iter().enumerate() {
            if ms < best[at] {
                best[at] = ms;
            }
        }
    }

    let mut readings = vec![
        Reading {
            name: "database",
            what: "open the state database and apply every migration",
            ms: best[0],
            budget_ms: 900.0,
        },
        Reading {
            name: "conversations written",
            what: "write 500 conversations with their first row",
            ms: best[1],
            budget_ms: 800.0,
        },
        Reading {
            name: "conversations listed",
            what: "list the 500 most recent conversations",
            ms: best[2],
            budget_ms: 150.0,
        },
        Reading {
            name: "events appended",
            what: "append 500 events to one conversation's log",
            ms: best[3],
            budget_ms: 400.0,
        },
        Reading {
            name: "conversation read",
            what: "read those 500 events back, as a window does",
            ms: best[4],
            budget_ms: 150.0,
        },
    ];

    // What the window felt like the last time it opened: the app writes this on its first frame, so
    // the number is a launch a person actually waited through, not a guess about one.
    //
    // Two readings, not one, because they are two different waits: reaching a devd (which starts one
    // when none is running) and going from there to something on screen. A warm devd answers in
    // milliseconds; a cold one has a whole registry and database to open first. Folding them into a
    // single number would hide whichever one is the problem.
    let window = startup(home);
    if let Some(w) = &window {
        let total = w["ms"].as_f64();
        let connect = w["connect_ms"].as_f64();
        if let Some(connect) = connect {
            readings.push(Reading {
                name: "devd ready",
                what: "wait for a running devd, starting one when there is none",
                ms: connect,
                budget_ms: 8_000.0,
            });
        }
        let drew = match (total, connect) {
            (Some(total), Some(connect)) => Some((total - connect).max(0.0)),
            (Some(total), None) => Some(total),
            _ => None,
        };
        if let Some(ms) = drew {
            readings.push(Reading {
                name: "window",
                what: "from a running devd to the first frame",
                ms,
                budget_ms: 2_500.0,
            });
        }
    }

    let ok = readings.iter().all(Reading::ok);
    Ok(json!({
        "ok": ok,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
        "measurements": readings.iter().map(Reading::json).collect::<Vec<_>>(),
        // Where the last launch's time went, when the window wrote it down.
        "window": window,
        "note": "Budgets are regression lines, not targets: several times what the work costs on an ordinary machine. Each reading is the best of three runs, because this is measured on a machine that is also doing other things. A miss is a change worth explaining, not a verdict on the machine.",
    }))
}

/// One pass over the work, timed. The order is the order a session goes through it.
fn run_once() -> Result<[f64; 5]> {
    let started = Instant::now();
    let store = Store::open_in_memory()?;
    store.migrate()?;
    let mut times = [0.0; 5];
    times[0] = since(started);

    let started = Instant::now();
    let mut thread = String::new();
    for n in 0..ROWS {
        let t = store.create_thread(&format!("conversation {n}"), None, None, None)?;
        if n == 0 {
            thread = t.id.clone();
        }
    }
    times[1] = since(started);

    let started = Instant::now();
    let listed = store.list_threads(ROWS as i64)?;
    times[2] = since(started);
    debug_assert_eq!(listed.len(), ROWS);

    let started = Instant::now();
    for n in 0..ROWS {
        store.append_event(&thread, "user", &json!({ "text": format!("message {n}") }))?;
    }
    times[3] = since(started);

    let started = Instant::now();
    let read = store.list_events(&thread, 0, 1000)?;
    times[4] = since(started);
    debug_assert_eq!(read.len(), ROWS);

    Ok(times)
}

fn since(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// The last launch's first-frame record, when the window wrote one down.
fn startup(home: &std::path::Path) -> Option<Value> {
    let text = std::fs::read_to_string(home.join("startup.json")).ok()?;
    let record: Value = serde_json::from_str(&text).ok()?;
    let ms = record["ms"].as_f64()?;
    (ms.is_finite() && ms >= 0.0).then_some(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_devd_measures_is_inside_its_budget() {
        // A folder with nothing in it: the window has not written a launch here, so the harness
        // measures its own work and says nothing about a window.
        let v = measure(std::path::Path::new(".")).expect("measuring does not fail");
        let rows = v["measurements"].as_array().expect("readings");
        assert!(rows.len() >= 5, "five readings at least: {v}");
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
