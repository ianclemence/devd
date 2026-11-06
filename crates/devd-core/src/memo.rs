//! Short-lived answers to expensive questions about the machine.
//!
//! `health.status`, `runtime.list`, `toolchain.list` and `setup.requirements`
//! each probe the machine: they run executables and open ports, which takes
//! seconds. Every screen and every poll asks, often several at once, and the
//! probes then slow each other down. Two rules keep that cheap without making
//! the answers stale:
//!
//! - **One probe at a time per question.** A caller that arrives while the
//!   question is being answered waits for that answer instead of starting a
//!   second probe.
//! - **An answer lasts until something changes.** It is kept for a few
//!   seconds, and any capability that changes anything (`bump`) discards every
//!   kept answer, so a service started a moment ago is never reported as down.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::Result;

static GENERATION: AtomicU64 = AtomicU64::new(0);

struct Entry {
    at: Instant,
    generation: u64,
    value: Value,
}

fn table() -> &'static Mutex<HashMap<String, Entry>> {
    static T: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

fn gate(key: &str) -> Arc<Mutex<()>> {
    static G: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let gates = G.get_or_init(|| Mutex::new(HashMap::new()));
    let mut gates = gates.lock().unwrap_or_else(|e| e.into_inner());
    gates.entry(key.to_string()).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
}

/// Something changed: every kept answer is out of date.
pub fn bump() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

fn fresh(key: &str, ttl: Duration) -> Option<Value> {
    let now = GENERATION.load(Ordering::SeqCst);
    let table = table().lock().unwrap_or_else(|e| e.into_inner());
    table
        .get(key)
        .filter(|e| e.generation == now && e.at.elapsed() < ttl)
        .map(|e| e.value.clone())
}

/// The answer to `key`, from the last probe if it is recent and nothing has
/// changed since, otherwise from `probe` (run by one caller at a time).
pub fn cached(key: &str, ttl: Duration, probe: impl FnOnce() -> Result<Value>) -> Result<Value> {
    if let Some(v) = fresh(key, ttl) {
        return Ok(v);
    }
    let gate = gate(key);
    let _one = gate.lock().unwrap_or_else(|e| e.into_inner());
    // Whoever held the gate may have just answered.
    if let Some(v) = fresh(key, ttl) {
        return Ok(v);
    }
    let generation = GENERATION.load(Ordering::SeqCst);
    let value = probe()?;
    table().lock().unwrap_or_else(|e| e.into_inner()).insert(
        key.to_string(),
        Entry { at: Instant::now(), generation, value: value.clone() },
    );
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    // `bump` is process-wide, so these tests take turns rather than invalidating each other.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn a_recent_answer_is_reused_and_a_change_discards_it() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let calls = AtomicUsize::new(0);
        let probe = || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Value::from(calls.load(Ordering::SeqCst)))
        };
        let ttl = Duration::from_secs(30);
        assert_eq!(cached("memo-test", ttl, probe).unwrap(), Value::from(1));
        assert_eq!(cached("memo-test", ttl, probe).unwrap(), Value::from(1), "reused");
        bump();
        assert_eq!(cached("memo-test", ttl, probe).unwrap(), Value::from(2), "a change discards it");
    }

    #[test]
    fn concurrent_callers_share_one_probe() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let calls = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let calls = calls.clone();
                std::thread::spawn(move || {
                    cached("memo-shared", Duration::from_secs(30), || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(80));
                        Ok(Value::Bool(true))
                    })
                    .unwrap()
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
