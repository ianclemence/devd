//! How much a run, and a task, may use.
//!
//! A run that cannot be stopped by its own cost is not under anyone's control. A person may give a run a ceiling on what it
//! spends and on how many model turns its agents take; devd tells them as it nears it and stops, pausing rather than
//! failing, when it is reached, so the work already done is kept and raising the ceiling continues it. Separately, one task
//! may not grow without limit inside one conversation: past a share of what a conversation can usefully hold, the agent is
//! asked to commit what works and hand over what remains, and a fresh agent is the one to continue.
//!
//! Only the arithmetic is here, so the rules can be read in one place and tested.

use devd_core::{RunBudget, RunUsage};

/// Tokens read by one task's conversation after which it is asked to wrap up. Past this a conversation is mostly its own
/// history, each turn costs more, and the agent reasons worse over it.
pub const TASK_WRAP_INPUT_TOKENS: i64 = 600_000;

/// Tokens written by one task's conversation after which it is asked to wrap up.
pub const TASK_WRAP_OUTPUT_TOKENS: i64 = 160_000;

/// The share of a ceiling at which the person is told that it is near.
pub const NEAR: f64 = 0.8;

/// Where a run stands against its ceilings.
#[derive(Debug, Clone, PartialEq)]
pub enum Standing {
    /// No ceiling is set, or the run is well inside it.
    Within,
    /// Past [`NEAR`] of a ceiling: what is left, in words.
    Near(String),
    /// A ceiling has been reached: which, in words.
    Reached(String),
}

/// A dollar amount as money reads: `$0.42`, `$12.50`.
pub fn dollars(amount: f64) -> String {
    format!("${amount:.2}")
}

/// Where a run stands.
pub fn standing(budget: RunBudget, usage: RunUsage) -> Standing {
    let mut near: Option<String> = None;
    if let Some(limit) = budget.usd.filter(|l| *l > 0.0) {
        if usage.cost_usd >= limit {
            return Standing::Reached(format!(
                "It has spent {} of the {} it was given",
                dollars(usage.cost_usd),
                dollars(limit)
            ));
        }
        if usage.cost_usd >= limit * NEAR {
            near = Some(format!(
                "{} of {} spent",
                dollars(usage.cost_usd),
                dollars(limit)
            ));
        }
    }
    if let Some(limit) = budget.turns.filter(|l| *l > 0) {
        if usage.turns >= limit {
            return Standing::Reached(format!(
                "Its agents have taken {} of the {limit} model turns it was given",
                usage.turns
            ));
        }
        if usage.turns as f64 >= limit as f64 * NEAR && near.is_none() {
            near = Some(format!("{} of {limit} model turns taken", usage.turns));
        }
    }
    match near {
        Some(words) => Standing::Near(words),
        None => Standing::Within,
    }
}

/// Whether one task's conversation has grown past what it should.
pub fn task_over(input_tokens: i64, output_tokens: i64) -> bool {
    input_tokens >= TASK_WRAP_INPUT_TOKENS || output_tokens >= TASK_WRAP_OUTPUT_TOKENS
}

/// What a task is told when it has grown past what it should: finish cleanly, and say what is left.
pub fn wrap_up_message(input_tokens: i64, output_tokens: i64) -> String {
    format!(
        "This task has used a large share of what one conversation can usefully hold ({input_tokens} tokens read, {output_tokens} written). Stop adding to it. Commit what works. Then call team.report: with status \"done\" if the acceptance criteria are met and project.verify passes, or with status \"blocked\" and a handoff (what is finished, what remains, and the first thing to do next, with the files involved) so that a fresh agent can continue from your notes without reading this conversation."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(cost: f64, turns: i64) -> RunUsage {
        RunUsage {
            cost_usd: cost,
            input_tokens: 0,
            output_tokens: 0,
            turns,
        }
    }

    #[test]
    fn a_run_with_no_ceiling_is_always_within_it() {
        assert_eq!(
            standing(RunBudget::default(), usage(500.0, 100_000)),
            Standing::Within
        );
    }

    #[test]
    fn the_person_is_told_as_a_ceiling_nears_and_the_run_stops_at_it() {
        let budget = RunBudget {
            usd: Some(2.0),
            turns: None,
        };
        assert_eq!(standing(budget, usage(1.0, 0)), Standing::Within);
        assert_eq!(
            standing(budget, usage(1.7, 0)),
            Standing::Near("$1.70 of $2.00 spent".into())
        );
        assert_eq!(
            standing(budget, usage(2.0, 0)),
            Standing::Reached("It has spent $2.00 of the $2.00 it was given".into())
        );
    }

    #[test]
    fn turns_are_a_ceiling_of_their_own_and_either_one_stops_the_run() {
        let budget = RunBudget {
            usd: Some(10.0),
            turns: Some(60),
        };
        assert!(matches!(
            standing(budget, usage(0.5, 49)),
            Standing::Near(_)
        ));
        assert!(
            matches!(standing(budget, usage(0.5, 60)), Standing::Reached(w) if w.contains("60"))
        );
        assert!(
            matches!(standing(budget, usage(10.5, 3)), Standing::Reached(_)),
            "the money ran out first"
        );
    }

    #[test]
    fn a_ceiling_of_nothing_is_not_a_ceiling() {
        assert_eq!(
            standing(
                RunBudget {
                    usd: Some(0.0),
                    turns: Some(0)
                },
                usage(3.0, 30)
            ),
            Standing::Within
        );
    }

    #[test]
    fn a_task_is_asked_to_wrap_up_when_its_conversation_has_grown_and_not_before() {
        assert!(!task_over(120_000, 20_000));
        assert!(task_over(TASK_WRAP_INPUT_TOKENS, 0));
        assert!(task_over(0, TASK_WRAP_OUTPUT_TOKENS));
        let words = wrap_up_message(700_000, 90_000);
        assert!(
            words.contains("team.report")
                && words.contains("fresh agent")
                && words.contains("700000")
        );
    }
}
