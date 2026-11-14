//! A goal: keep working until an objective is met.
//!
//! Normally a turn ends when the model stops calling tools. With a goal set on a conversation, devd does not
//! take that for the end: it asks the model whether the objective is met, and the model either calls
//! `goal_done` with the evidence or takes the next step. The person can pause the goal (the turn then ends as
//! usual), resume it, or clear it.
//!
//! A goal is bounded on purpose. A model that never finishes would otherwise keep a conversation, and a bill,
//! going: so it is asked at most [`MAX_STEPS`] times, and stops when the conversation has spent [`MAX_COST_USD`]
//! since the goal was set. Either way the person is told, in the conversation, why it stopped.

use devd_core::state::Store;
use serde::{Deserialize, Serialize};

/// The most times devd sends a conversation back to work on its goal.
pub const MAX_STEPS: u32 = 25;
/// The most a conversation may spend, in US dollars, while it works toward one goal.
pub const MAX_COST_USD: f64 = 5.0;

/// A goal on a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    /// What is to be achieved, in the person's words.
    pub objective: String,
    /// `active`, `paused`, `met` or `stopped`.
    pub state: String,
    /// How many times the conversation has been sent back to work on it.
    pub steps: u32,
    /// What the conversation had cost when the goal was set.
    pub cost_at_start: f64,
    /// When it was set.
    pub started_at: String,
    /// The evidence it was met, or why it stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What to do when the model stops.
#[derive(Debug, Clone, PartialEq)]
pub enum Next {
    /// No goal is being worked on: the turn ends as it would.
    End,
    /// Send the model back to work, with this said to it.
    Continue(String),
    /// Give up, for this reason, and tell the person.
    Stop(String),
}

fn key(thread: &str) -> String {
    format!("goal:{thread}")
}

/// The goal on a conversation, if there is one.
pub fn get(store: &Store, thread: &str) -> Option<Goal> {
    let text = store.get_setting(&key(thread)).ok().flatten()?;
    serde_json::from_str(&text).ok()
}

/// Keep a goal.
pub fn put(store: &Store, thread: &str, goal: &Goal) -> devd_core::Result<()> {
    store.set_setting(&key(thread), &serde_json::to_string(goal).unwrap_or_default())
}

/// Take a goal away.
pub fn clear(store: &Store, thread: &str) -> devd_core::Result<()> {
    store.set_setting(&key(thread), "")
}

/// A new goal, active, on a conversation that has cost `cost_now` so far.
pub fn new(objective: &str, cost_now: f64) -> Goal {
    Goal {
        objective: objective.trim().to_string(),
        state: "active".into(),
        steps: 0,
        cost_at_start: cost_now,
        started_at: chrono::Utc::now().to_rfc3339(),
        note: None,
    }
}

/// What to do now that the model has stopped, given the goal and what the conversation has cost.
pub fn next(goal: Option<&Goal>, cost_now: f64) -> Next {
    let Some(goal) = goal.filter(|g| g.state == "active") else {
        return Next::End;
    };
    if goal.steps >= MAX_STEPS {
        return Next::Stop(format!("it was sent back to work {MAX_STEPS} times and the goal is still open"));
    }
    let spent = (cost_now - goal.cost_at_start).max(0.0);
    if spent >= MAX_COST_USD {
        return Next::Stop(format!("the conversation has spent ${spent:.2} on it, over the ${MAX_COST_USD:.2} a goal may use"));
    }
    Next::Continue(format!(
        "[devd] The goal is still open: {}\n\nIf it is met, call goal_done with the evidence: what you ran or read that shows it. If it is not, take the next concrete step now. Do not ask the person whether to continue; if you are blocked on something only they can give you, say exactly what, and stop.",
        goal.objective
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_goal_or_a_paused_one_the_turn_ends_as_it_would() {
        assert_eq!(next(None, 0.0), Next::End);
        let mut g = new("make the tests pass", 0.0);
        g.state = "paused".into();
        assert_eq!(next(Some(&g), 0.0), Next::End);
        g.state = "met".into();
        assert_eq!(next(Some(&g), 0.0), Next::End);
    }

    #[test]
    fn an_active_goal_sends_the_model_back_with_the_objective_and_how_to_finish() {
        let g = new("  make the tests pass  ", 1.0);
        match next(Some(&g), 1.2) {
            Next::Continue(text) => {
                assert!(text.contains("make the tests pass"), "{text}");
                assert!(text.contains("goal_done"), "{text}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_goal_that_never_finishes_stops_at_its_step_limit_and_at_its_spending_limit() {
        let mut g = new("x", 2.0);
        g.steps = MAX_STEPS;
        assert!(matches!(next(Some(&g), 2.0), Next::Stop(why) if why.contains("25 times")));
        g.steps = 3;
        assert!(matches!(next(Some(&g), 2.0 + MAX_COST_USD), Next::Stop(why) if why.contains("$5.00")));
        // What the conversation spent before the goal is not held against it.
        assert!(matches!(next(Some(&g), 2.0 + MAX_COST_USD - 0.01), Next::Continue(_)));
    }

    #[test]
    fn a_goal_is_kept_per_conversation_and_can_be_taken_away() {
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        assert!(get(&store, "t1").is_none());
        let g = new("ship it", 0.0);
        put(&store, "t1", &g).unwrap();
        assert_eq!(get(&store, "t1"), Some(g.clone()));
        assert!(get(&store, "t2").is_none(), "another conversation has none");
        clear(&store, "t1").unwrap();
        assert!(get(&store, "t1").is_none());
    }
}
