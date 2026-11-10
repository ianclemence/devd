//! How the team works, measured from what it left behind.
//!
//! Every number here comes from a task's record: its final state, how many
//! attempts it needed, how long it ran, the evidence devd recorded and the review
//! events. Output volume is never a score: a role that finishes many tasks that
//! were rejected or reworked reads as exactly that.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use devd_core::store_team::TeamTask;
use devd_core::Result;

use devd_core::capability::Ctx;

#[derive(Default)]
struct Tally {
    tasks: i64,
    accepted: i64,
    rejected: i64,
    abandoned: i64,
    open: i64,
    rework: i64,
    changes_requested: i64,
    verified_runs: i64,
    failed_runs: i64,
    seconds: Vec<i64>,
}

fn seconds(t: &TeamTask) -> Option<i64> {
    let start = chrono::DateTime::parse_from_rfc3339(t.started_at.as_deref()?).ok()?;
    let end = chrono::DateTime::parse_from_rfc3339(t.finished_at.as_deref()?).ok()?;
    let s = (end - start).num_seconds();
    (s >= 0).then_some(s)
}

fn median(v: &mut [i64]) -> Option<i64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

fn tally(t: &TeamTask, by_task: &BTreeMap<(String, String), i64>, into: &mut Tally) {
    into.tasks += 1;
    match t.state.as_str() {
        "done" => into.accepted += 1,
        "unverified" | "failed" | "blocked" => into.rejected += 1,
        "cancelled" | "skipped" => into.abandoned += 1,
        _ => into.open += 1,
    }
    into.rework += (t.attempts - 1).max(0);
    into.changes_requested += by_task.get(&(t.id.clone(), "changes_requested".into())).copied().unwrap_or(0);
    if let Some(items) = t.evidence.as_array() {
        for f in items {
            if f["kind"] == "verify" {
                if f["ok"] == true {
                    into.verified_runs += 1;
                } else {
                    into.failed_runs += 1;
                }
            }
        }
    }
    if let Some(s) = seconds(t) {
        into.seconds.push(s);
    }
}

fn report(name: &str, mut t: Tally) -> Value {
    let settled = t.accepted + t.rejected + t.abandoned;
    let runs = t.verified_runs + t.failed_runs;
    json!({
        "name": name,
        "tasks": t.tasks,
        "accepted": t.accepted,
        "rejected": t.rejected,
        "abandoned": t.abandoned,
        "open": t.open,
        // Accepted on proof, out of the tasks that ended; open work is not counted either way.
        "acceptance_rate": if settled > 0 { Some((t.accepted as f64) / (settled as f64)) } else { None },
        // Extra attempts per task: how much work had to be redone.
        "rework_per_task": if t.tasks > 0 { Some((t.rework as f64) / (t.tasks as f64)) } else { None },
        "changes_requested": t.changes_requested,
        "verification_pass_rate": if runs > 0 { Some((t.verified_runs as f64) / (runs as f64)) } else { None },
        "verification_runs": runs,
        "median_seconds": median(&mut t.seconds),
    })
}

/// The team's record: overall, per role, and per project.
pub fn stats(ctx: &Ctx) -> Result<Value> {
    let tasks = ctx.store.all_team_tasks()?;
    let counts = ctx.store.team_event_counts(&["changes_requested", "rejected", "retry", "retrying"])?;
    let by_task: BTreeMap<(String, String), i64> = counts.into_iter().map(|(t, k, n)| ((t, k), n)).collect();
    let runs = ctx.store.list_runs(1000).unwrap_or_default();
    let project_of: BTreeMap<String, String> = runs
        .iter()
        .map(|r| (r.id.clone(), r.project.clone().unwrap_or_default()))
        .collect();

    let mut all = Tally::default();
    let mut roles: BTreeMap<String, Tally> = BTreeMap::new();
    let mut projects: BTreeMap<String, Tally> = BTreeMap::new();
    for t in tasks.iter().filter(|t| t.role != "planner") {
        tally(t, &by_task, &mut all);
        tally(t, &by_task, roles.entry(t.role.clone()).or_default());
        let project = project_of.get(&t.run_id).cloned().unwrap_or_default();
        tally(t, &by_task, projects.entry(project).or_default());
    }
    let run_states: BTreeMap<String, i64> = runs.iter().fold(BTreeMap::new(), |mut m, r| {
        *m.entry(r.state.clone()).or_insert(0) += 1;
        m
    });
    Ok(json!({
        "team": report("team", all),
        "roles": roles.into_iter().map(|(k, v)| report(&k, v)).collect::<Vec<_>>(),
        "projects": projects.into_iter().map(|(k, v)| report(&k, v)).collect::<Vec<_>>(),
        "runs": run_states,
    }))
}

/// Tokens by model, from the conversations that used them.
pub fn usage(ctx: &Ctx) -> Result<Value> {
    let threads = ctx.store.list_threads(100_000)?;
    let tasks = ctx.store.all_team_tasks().unwrap_or_default();
    let role_of: BTreeMap<String, String> = tasks
        .iter()
        .filter_map(|t| t.thread_id.clone().map(|id| (id, t.role.clone())))
        .collect();
    // Spend is an estimate, not a provider invoice: it uses the catalogue's
    // current price for a model against the tokens that model actually reported.
    // A model with no known price stays unpriced and is counted as such.
    let price: BTreeMap<String, (f64, f64)> = ctx
        .store
        .list_catalogue(None)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| Some((m.model_id.to_lowercase(), (m.input_price?, m.output_price?))))
        .collect();

    #[derive(Default)]
    struct U {
        conversations: i64,
        input: i64,
        output: i64,
        cache_read: i64,
        cache_write: i64,
        cost: f64,
        priced: i64,
        unpriced: i64,
    }
    // A conversation that did not name a model used its provider's default. Name
    // that model here so the row can be priced and read, instead of every such
    // conversation collecting under the opaque label "default model".
    let default_model_of: BTreeMap<String, String> = ctx
        .store
        .list_providers()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| Some((p.id, p.default_model?)))
        .collect();

    let mut by_model: BTreeMap<String, U> = BTreeMap::new();
    let mut by_who: BTreeMap<String, U> = BTreeMap::new();
    let mut top: Vec<(i64, Value)> = Vec::new();
    for t in &threads {
        let model = t
            .model
            .clone()
            .or_else(|| t.provider_id.as_ref().and_then(|id| default_model_of.get(id)).cloned())
            .unwrap_or_else(|| "default model".into());
        let who = role_of.get(&t.id).cloned().unwrap_or_else(|| "assistant".into());
        let rate = price.get(&model.to_lowercase()).copied();
        // Prefer what was recorded when the call was made; fall back to the
        // catalogue price for conversations recorded before per-call cost.
        let cost = t.cost_usd.or_else(|| {
            rate.map(|(i, o)| (i * t.input_tokens as f64 + o * t.output_tokens as f64) / 1_000_000.0)
        });
        for (map, key) in [(&mut by_model, model.clone()), (&mut by_who, who.clone())] {
            let u = map.entry(key).or_default();
            u.conversations += 1;
            u.input += t.input_tokens;
            u.output += t.output_tokens;
            u.cache_read += t.cache_read_tokens;
            u.cache_write += t.cache_write_tokens;
            match cost {
                Some(c) => {
                    u.cost += c;
                    u.priced += 1;
                }
                None => u.unpriced += 1,
            }
        }
        top.push((t.input_tokens + t.output_tokens, json!({
            "id": t.id, "title": t.title, "who": who, "model": model,
            "input": t.input_tokens, "output": t.output_tokens,
            "cost_usd": cost,
        })));
    }
    top.sort_by_key(|x| std::cmp::Reverse(x.0));
    let rows = |m: BTreeMap<String, U>| -> Vec<Value> {
        let mut v: Vec<Value> = m
            .into_iter()
            .map(|(k, u)| json!({
                "name": k,
                "conversations": u.conversations,
                "input": u.input,
                "output": u.output,
                "cache_read": u.cache_read,
                "cache_write": u.cache_write,
                "cost_usd": if u.priced > 0 { Some(u.cost) } else { None },
                "priced_conversations": u.priced,
                "unpriced_conversations": u.unpriced,
            }))
            .collect();
        v.sort_by(|a, b| (b["input"].as_i64().unwrap_or(0) + b["output"].as_i64().unwrap_or(0)).cmp(&(a["input"].as_i64().unwrap_or(0) + a["output"].as_i64().unwrap_or(0))));
        v
    };
    let known: Vec<f64> = by_model.values().filter(|u| u.priced > 0).map(|u| u.cost).collect();
    let unpriced: i64 = by_model.values().map(|u| u.unpriced).sum();
    Ok(json!({
        "by_model": rows(by_model),
        "by_who": rows(by_who),
        "heaviest": top.into_iter().take(8).map(|(_, v)| v).collect::<Vec<_>>(),
        "total_cost_usd": if known.is_empty() { None } else { Some(known.iter().sum::<f64>()) },
        "unpriced_conversations": unpriced,
        "note": "Tokens and cache use as each provider reported them. Spend is what was recorded per call from the price in force; a conversation recorded before that shows the catalogue estimate. It remains an estimate of the provider's bill, not the invoice.",
    }))
}
