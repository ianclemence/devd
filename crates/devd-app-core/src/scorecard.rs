//! A scorecard for agents and models, built from what devd recorded.
//!
//! This is a person's view of work, not a leaderboard for its own sake. A task
//! counts only when devd saw the evidence; output volume is never a score. The
//! formula is deliberately small and printed beside the number, so a rank can
//! be argued with. A role with fewer than three settled tasks is shown but not
//! ranked: one accepted task is not a record.

use serde_json::Value;

/// One agent or model, measured.
#[derive(Debug, Clone, PartialEq)]
pub struct Scorecard {
    /// The role id, or the model id.
    pub name: String,
    /// Every task it was given.
    pub tasks: i64,
    /// Tasks that ended (accepted, rejected or abandoned).
    pub settled: i64,
    /// Tasks accepted on proof.
    pub accepted: i64,
    /// Tasks still open.
    pub open: i64,
    /// Accepted out of settled, when there is a record.
    pub acceptance_rate: Option<f64>,
    /// Extra attempts per task.
    pub rework_per_task: Option<f64>,
    /// Checks that passed, out of checks that ran.
    pub verification_pass_rate: Option<f64>,
    /// Review passes that asked for a change.
    pub changes_requested: i64,
    /// Median seconds from start to finish.
    pub median_seconds: Option<i64>,
    /// Estimated spend, when the model has a price.
    pub cost_usd: Option<f64>,
    /// Spend divided by tasks accepted on proof.
    pub cost_per_accepted: Option<f64>,
    /// Conversations that used this model or member.
    pub conversations: i64,
    /// Input plus output tokens.
    pub tokens: u64,
    /// Conversations that ended in an error.
    pub errors: i64,
    /// Errors out of conversations.
    pub error_rate: Option<f64>,
    /// Share of input read from the prompt cache.
    pub cache_hit_rate: Option<f64>,
    /// 0–100, or `None` when there is not enough record to rank.
    pub score: Option<f64>,
    /// 1 is best. `None` when not ranked.
    pub rank: Option<usize>,
}

impl Scorecard {
    /// Why it has no rank, in a few words.
    pub fn note(&self) -> &'static str {
        if self.rank.is_some() {
            ""
        } else if self.name == "not recorded" {
            "model not recorded"
        } else if self.settled < 3 {
            "too new to rank"
        } else {
            ""
        }
    }
}

fn number(v: &Value, key: &str) -> i64 {
    v[key].as_i64().unwrap_or(0)
}

fn rate(v: &Value, key: &str) -> Option<f64> {
    v[key].as_f64()
}

fn tokens(row: &Value) -> u64 {
    row["input"].as_u64().unwrap_or(0) + row["output"].as_u64().unwrap_or(0)
}

/// The quality part of the score, 0–100. Every part is a fact devd recorded:
/// accepted on proof, checks passed, and work that had to be redone. Output
/// volume is not in it.
fn quality(acceptance: Option<f64>, verification: Option<f64>, rework: Option<f64>, changes: i64, tasks: i64) -> f64 {
    let acceptance = acceptance.unwrap_or(0.0).clamp(0.0, 1.0);
    // With no check recorded, acceptance is the only proof there is; do not
    // reward a missing check as if it were a pass.
    let verification = verification.unwrap_or(acceptance).clamp(0.0, 1.0);
    let rework = rework.unwrap_or(0.0).max(0.0);
    let rework_part = 1.0 - (rework / 2.0).clamp(0.0, 1.0);
    let changes_part = if tasks > 0 { (changes as f64 / tasks as f64).clamp(0.0, 1.0) } else { 0.0 };
    (acceptance * 55.0 + verification * 25.0 + rework_part * 20.0 - changes_part * 10.0).clamp(0.0, 100.0)
}

fn from_row(row: &Value, usage: Option<&Value>) -> Scorecard {
    let tasks = number(row, "tasks");
    let settled = number(row, "settled");
    let accepted = number(row, "accepted");
    let acceptance = rate(row, "acceptance_rate");
    let verification = rate(row, "verification_pass_rate");
    let rework = rate(row, "rework_per_task");
    let changes = number(row, "changes_requested");
    let cost = usage.and_then(|u| u["cost_usd"].as_f64()).or_else(|| row["cost_usd"].as_f64());
    // A row that says the model was not recorded has no model to rank.
    let score = (settled >= 3 && row["name"].as_str() != Some("not recorded")).then(|| quality(acceptance, verification, rework, changes, tasks));
    Scorecard {
        name: row["name"].as_str().unwrap_or_default().to_string(),
        tasks,
        settled,
        accepted,
        open: number(row, "open"),
        acceptance_rate: acceptance,
        rework_per_task: rework,
        verification_pass_rate: verification,
        changes_requested: changes,
        median_seconds: row["median_seconds"].as_i64(),
        cost_usd: cost,
        cost_per_accepted: match (cost, accepted) {
            (Some(c), n) if n > 0 => Some(c / n as f64),
            _ => None,
        },
        conversations: usage.and_then(|u| u["conversations"].as_i64()).unwrap_or(0),
        tokens: usage.map(tokens).unwrap_or(0),
        errors: usage.and_then(|u| u["errors"].as_i64()).unwrap_or(0),
        error_rate: usage.and_then(|u| u["error_rate"].as_f64()),
        cache_hit_rate: usage.and_then(|u| u["cache_hit_rate"].as_f64()),
        score,
        rank: None,
    }
}

/// Put the ranked first, best score first, and give each a place.
fn place(mut rows: Vec<Scorecard>) -> Vec<Scorecard> {
    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.accepted.cmp(&a.accepted))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut rank = 0;
    for row in &mut rows {
        if row.score.is_some() {
            rank += 1;
            row.rank = Some(rank);
        }
    }
    rows
}

fn usage_row<'a>(usage: &'a Value, pointer: &str, name: &str) -> Option<&'a Value> {
    usage[pointer].as_array()?.iter().find(|row| row["name"].as_str() == Some(name))
}

/// Agents, from `team.stats.roles` and `models.usage.by_who`.
pub fn agents(stats: &Value, usage: &Value) -> Vec<Scorecard> {
    let rows: Vec<Scorecard> = stats["roles"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|row| number(row, "tasks") > 0)
        .map(|row| {
            let name = row["name"].as_str().unwrap_or_default();
            from_row(row, usage_row(usage, "by_who", name))
        })
        .collect();
    place(rows)
}

/// Models, from `team.stats.models` and `models.usage.by_model`. A model that
/// did work but has no finished task yet is still shown, so its cost is visible.
pub fn models(stats: &Value, usage: &Value) -> Vec<Scorecard> {
    let mut rows: Vec<Scorecard> = stats["models"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|row| number(row, "tasks") > 0)
        .map(|row| {
            let name = row["name"].as_str().unwrap_or_default();
            from_row(row, usage_row(usage, "by_model", name))
        })
        .collect();
    for row in usage["by_model"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let name = row["name"].as_str().unwrap_or_default();
        if rows.iter().any(|known| known.name == name) {
            continue;
        }
        rows.push(Scorecard {
            name: name.to_string(),
            tasks: 0,
            settled: 0,
            accepted: 0,
            open: 0,
            acceptance_rate: None,
            rework_per_task: None,
            verification_pass_rate: None,
            changes_requested: 0,
            median_seconds: None,
            cost_usd: row["cost_usd"].as_f64(),
            cost_per_accepted: None,
            conversations: row["conversations"].as_i64().unwrap_or(0),
            tokens: tokens(row),
            errors: row["errors"].as_i64().unwrap_or(0),
            error_rate: row["error_rate"].as_f64(),
            cache_hit_rate: row["cache_hit_rate"].as_f64(),
            score: None,
            rank: None,
        });
    }
    place(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn role(name: &str, tasks: i64, accepted: i64, acceptance: f64, rework: f64, verify: f64) -> Value {
        json!({
            "name": name, "tasks": tasks, "settled": tasks, "accepted": accepted, "open": 0,
            "acceptance_rate": acceptance, "rework_per_task": rework,
            "verification_pass_rate": verify, "changes_requested": 0,
            "median_seconds": 60,
        })
    }

    #[test]
    fn a_role_needs_three_settled_tasks_before_it_is_ranked() {
        let stats = json!({ "roles": [role("implementer", 2, 2, 1.0, 0.0, 1.0), role("backend", 5, 4, 0.8, 0.2, 0.9)] });
        let rows = agents(&stats, &json!({}));
        let implementer = rows.iter().find(|r| r.name == "implementer").unwrap();
        assert_eq!(implementer.rank, None);
        assert_eq!(implementer.note(), "too new to rank");
        let backend = rows.iter().find(|r| r.name == "backend").unwrap();
        assert_eq!(backend.rank, Some(1));
        assert!(backend.score.unwrap() > 60.0);
    }

    #[test]
    fn quality_falls_when_work_is_redone_or_rejected() {
        let stats = json!({ "roles": [
            role("careful", 10, 10, 1.0, 0.0, 1.0),
            role("reworked", 10, 6, 0.6, 1.0, 0.7),
        ]});
        let rows = agents(&stats, &json!({}));
        assert_eq!(rows[0].name, "careful");
        assert!(rows[0].score.unwrap() > rows[1].score.unwrap());
        assert!(rows[1].score.unwrap() < 70.0);
    }

    #[test]
    fn cost_is_shown_per_accepted_task_and_a_model_without_price_has_no_figure() {
        let stats = json!({ "models": [role("deepseek-chat", 4, 3, 0.75, 0.3, 0.8)] });
        let usage = json!({ "by_model": [
            { "name": "deepseek-chat", "input": 1000, "output": 200, "cost_usd": 3.0, "cache_hit_rate": 0.5 },
            { "name": "mystery", "input": 10, "output": 2, "cost_usd": null, "cache_hit_rate": null },
        ]});
        let rows = models(&stats, &usage);
        let deepseek = rows.iter().find(|r| r.name == "deepseek-chat").unwrap();
        assert_eq!(deepseek.tokens, 1200);
        assert_eq!(deepseek.cost_per_accepted, Some(1.0));
        assert_eq!(deepseek.cache_hit_rate, Some(0.5));
        let mystery = rows.iter().find(|r| r.name == "mystery").unwrap();
        assert_eq!(mystery.cost_usd, None);
        assert_eq!(mystery.rank, None);
        assert_eq!(mystery.score, None);
    }
}
