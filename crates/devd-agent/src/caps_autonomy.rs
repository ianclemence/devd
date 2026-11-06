//! The gate on devd's own autonomy: the work devd may do, and the rules a
//! project wrote for when it should think about doing it.
//!
//! Two capabilities decide everything here:
//!
//! * `team.suggest` - how work devd *proposes* is recorded. It writes one row and
//!   no run; a person turns it into a run with `team.accept_suggestion`, which
//!   starts an ordinary run that still waits for plan approval.
//! * `intake.*` - the rules a project wrote. A person writes them; the daemon
//!   evaluates them; nothing they start escapes the ordinary approval model.
//!
//! Keeping them in one capability group means the window, the terminal, the CLI,
//! MCP and the daemon all read and write the same rows, and that a reviewer can
//! see the whole of devd's proactive surface in one place.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::{util, DevdError, IntakeRule, Registry, Result, Suggestion};

use crate::intake;
use crate::roles;
use crate::team::{StartRequest, Team};

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
struct NoAutomationArgs {}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct AutomationSet {
    /// A short name, lower-case.
    name: String,
    /// The event this accepts: `github.pull_request`, `github.push` or a name
    /// your own sender uses.
    event: String,
    /// Optional match conditions. `actions`, `repo`, `branch`, or `equals` with
    /// dot paths into the JSON body.
    #[serde(default)]
    filter: Option<Value>,
    /// What to do: `{ "type": "team", "project": "...", "goal": "..." }`,
    /// `{ "type": "chat", "message": "..." }` or
    /// `{ "type": "inbox", "subject": "...", "body": "..." }`.
    action: Value,
    /// A token to use. Absent: an existing hook keeps its token (so its sender keeps
    /// working) and a new hook gets a fresh one.
    #[serde(default)]
    token: Option<String>,
    /// True to replace the token: the old address stops working at once.
    #[serde(default)]
    rotate: Option<bool>,
    /// Default true.
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct AutomationRef {
    /// Rule name.
    name: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct SuggestArgs {
    /// Project the work is for (new work needs one).
    #[serde(default)]
    project: Option<String>,
    /// The team member who would do it. Defaults to the architect for planning
    /// work and the implementer for a change.
    #[serde(default)]
    role: Option<String>,
    /// One line, in the words a person would use.
    title: String,
    /// What you noticed that makes this worth doing: a failing check, an
    /// inconsistency, a risk. Not a wish; the observation.
    why: String,
    /// The steps you would take, one per line.
    #[serde(default)]
    plan: Vec<String>,
    /// How the work would be known to be done, one per line.
    #[serde(default)]
    accepts: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct SuggestionRef {
    /// Suggestion id.
    id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct SuggestList {
    /// Only these: `open`, `accepted`, `dismissed` or `expired`.
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct RuleWrite {
    /// Project directory.
    project: String,
    /// Short name, unique within the project.
    name: String,
    /// What to watch: `health.down`, `check.failed` or `schedule.missed`.
    trigger: String,
    /// How many times in a row before it counts (`min_failures`, default 1), how
    /// long to wait after starting a run (`cooldown_minutes`, default 60), and
    /// optionally the steps to show a person (`steps`).
    #[serde(default)]
    condition: Option<Value>,
    /// What the run should do.
    goal: String,
    /// A run template to use instead of the goal.
    #[serde(default)]
    template: Option<String>,
    /// The member who would do it.
    #[serde(default)]
    role: Option<String>,
    /// `observe` records what it would have done and does nothing else (the
    /// default); `auto` starts the run, which still waits for plan approval.
    #[serde(default)]
    mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct RuleRef {
    /// Project directory.
    project: String,
    /// Rule name.
    name: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct RulesQuery {
    /// Only this project's rules.
    #[serde(default)]
    project: Option<String>,
}

fn simple(id: &str, text: impl Into<String>) -> Result<Plan> {
    Ok(Plan::new(id, text))
}

fn suggestion_json(ctx: &Ctx, s: &Suggestion) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
    if let Some(r) = roles::get(&s.role) {
        v["role_title"] = json!(r.title);
        v["role_summary"] = json!(r.summary);
    }
    // What pressing the button would do, in the same words every surface uses.
    v["accept_last"] = json!(ctx.store.environment_mode() == "production");
    v
}

fn rule_json(r: &IntakeRule) -> Value {
    json!({
        "project": r.project,
        "name": r.name,
        "trigger": r.trigger,
        "trigger_text": intake::TRIGGERS.iter().find(|(t, _)| *t == r.trigger.as_str()).map(|(_, s)| *s).unwrap_or("an unknown event"),
        "condition": r.condition,
        "condition_text": intake::condition_text(r),
        "goal": r.goal,
        "template": r.template,
        "role": r.role,
        "mode": r.mode,
        "countdown": r.countdown,
        "min_failures": r.min_failures(),
        "fired_at": r.fired_at,
        "enabled": r.enabled,
    })
}

/// Register the suggestion and intake-rule capabilities.
pub fn register(reg: &mut Registry, team: Arc<Team>) -> Result<()> {
    reg.define::<SuggestArgs>("team.suggest", "Propose work instead of doing it")
    .description("Records a proposal for work that would move the project forward, with what made it worth doing and how it would be proved done. Nothing runs: the proposal is one row, shown to the person in the Inbox and the team room, and it becomes a run only if a person accepts it (team.accept_suggestion), which then waits for plan approval like any other run. Use this for the improvement you noticed while doing something else. Do not use it to ask for permission; ask in your reply instead.")
    .group("team")
    .tier(Tier::Write)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .handlers(
        |_, a| simple("team.suggest", format!("propose: {}", util::truncate(&a.title, 80))),
        |ctx, a| {
            let title = a.title.trim();
            let why = a.why.trim();
            if title.chars().count() < 6 {
                return Err(DevdError::invalid("a suggestion needs a title a person can act on"));
            }
            if why.chars().count() < 10 {
                return Err(DevdError::invalid("say what you noticed that makes this worth doing"));
            }
            let role = a.role.as_deref().unwrap_or("implementer").to_ascii_lowercase();
            if roles::get(&role).is_none() {
                return Err(DevdError::invalid(format!("`{role}` is not a member of the team; team.roles lists them")));
            }
            let plan: Vec<String> = a.plan.iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
            let accepts: Vec<String> = a.accepts.iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
            let project = a.project.as_deref().filter(|p| !p.trim().is_empty()).map(|p| {
                std::path::Path::new(p.trim()).canonicalize().map(|d| d.display().to_string()).unwrap_or_else(|_| p.trim().to_string())
            });
            // The same idea proposed twice while the first is still open is one
            // question, not two.
            if let Some(existing) = ctx.store.open_suggestion_titled(project.as_deref(), title)? {
                return Ok(Outcome::with_data("it was already proposed; nothing new was recorded", json!({ "id": existing.id, "duplicate": true })));
            }
            let s = ctx.store.add_suggestion(project.as_deref(), &role, title, why, &plan, &accepts, "manual", None)?;
            ctx.events.emit_with("team.suggestion", s.title.clone(), json!({ "id": s.id, "project": s.project }));
            Ok(Outcome::with_data("proposed; nothing runs unless a person accepts it", suggestion_json(ctx, &s)))
        },
    )?;

    reg.define::<SuggestList>("team.suggestions", "Read the proposals")
    .description("Work devd proposed and has not done, with what it noticed and how it would be proved done. Accepted and dismissed proposals are kept with the run they became.")
    .group("team")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| simple("team.suggestions", "read the proposals"),
        |ctx, a| {
            let rows: Vec<Value> = ctx.store.list_suggestions(a.state.as_deref(), 100)?.iter().map(|s| suggestion_json(ctx, s)).collect();
            let open = rows.iter().filter(|r| r["state"] == "open").count();
            Ok(Outcome::with_data(format!("{} open, {} in all", open, rows.len()), json!(rows)))
        },
    )?;

    {
        let t = team.clone();
        reg.define::<SuggestionRef>("team.accept_suggestion", "Turn a proposal into a run")
    .description("Starts an ordinary team run from a proposal: the planner plans it and a person still approves the plan before anything runs. The run records that devd proposed it and who accepted it.")
    .group("team")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .human_only()
    .dry_run()
    .handlers(
        |ctx, a| {
            let s = ctx.store.get_suggestion(&a.id)?.ok_or_else(|| DevdError::not_found("no such proposal"))?;
            Ok(Plan::new("team.accept_suggestion", format!("start a run from the proposal: {}", s.title))
                .effect("the planner plans it; nothing else runs until you approve the plan"))
        },
        move |ctx, a| {
                let s = ctx.store.get_suggestion(&a.id)?.ok_or_else(|| DevdError::not_found("no such proposal"))?;
                if s.state != "open" {
                    return Err(DevdError::conflict(format!("this proposal is {}", s.state)));
                }
                let project = s.project.as_deref().ok_or_else(|| DevdError::invalid("the proposal has no project; start the run yourself with team.start"))?;
                let done_when = if s.accepts.is_empty() {
                    Vec::new()
                } else {
                    s.accepts.clone()
                };
                let goal = if s.plan.is_empty() {
                    s.title.clone()
                } else {
                    format!("{}\n\nWhat prompted it: {}\n\nSteps devd would take:\n{}", s.title.clone(), s.why, s.plan.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n"))
                };
                let run = t.start(ctx, &StartRequest {
                    goal,
                    project: Some(project.to_string()),
                    new_project: None,
                    parallel: 2,
                    auto_approve: false,
                    done_when,
                    max_rounds: 2,
                })?;
                ctx.store.decide_suggestion(&s.id, true, Some(&run.id), &ctx.actor.label())?;
                ctx.events.emit_with("team.suggestion.accepted", s.title.clone(), json!({ "id": s.id, "run": run.id }));
                Ok(Outcome::with_data("started; the plan will wait for you", json!({ "run": run, "suggestion": s.id })))
            },
    )?;
    }

    reg.define::<SuggestionRef>("team.dismiss_suggestion", "Decline a proposal")
    .description("Records that this proposal is not wanted. devd keeps the record, so a rule that keeps proposing it shows as dismissed rather than nagging. Human only.")
    .group("team")
    .tier(Tier::Write)
    .effect(Effect::Process)
    .human_only()
    .dry_run()
    .handlers(
        |_, a| simple("team.dismiss_suggestion", format!("dismiss proposal {}", a.id)),
        |ctx, a| {
            let s = ctx.store.get_suggestion(&a.id)?.ok_or_else(|| DevdError::not_found("no such proposal"))?;
            ctx.store.decide_suggestion(&a.id, false, None, &ctx.actor.label())?;
            Ok(Outcome::ok(format!("dismissed: {}", s.title)))
        },
    )?;

    // ------------------------------------------------------------- intake

    reg.define::<RulesQuery>("intake.list", "The project's rules for starting a run")
    .description("Each written rule: which event it watches, how many times it must happen, how often it may start a run, what that run would do, and whether it is in observe mode (records what it would have done) or auto (starts the run, which still waits for plan approval).")
    .group("intake")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| simple("intake.list", "read the intake rules"),
        |ctx, a| {
            let rules = ctx.store.list_intake_rules(a.project.as_deref())?;
            Ok(Outcome::with_data(format!("{} rule(s)", rules.len()), json!({
                "triggers": intake::TRIGGERS.iter().map(|(t, d)| json!({ "trigger": t, "detail": d })).collect::<Vec<_>>(),
                "rules": rules.iter().map(rule_json).collect::<Vec<_>>(),
            })))
        },
    )?;

    reg.define::<RuleWrite>("intake.set", "Write a rule for starting a run")
    .description("Adds or changes one rule for a project: when to consider a run. A person writes this, because it decides what devd may start without being asked. Starts in observe mode unless mode is given; auto still waits for plan approval and is ignored in production.")
    .group("intake")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .human_only()
    .dry_run()
    .handlers(
        |_, a| Ok(Plan::new("intake.set", format!("rule `{}` for {}", a.name, a.project))
            .effect(format!("devd would start a run when {}", intake::TRIGGERS.iter().find(|(t, _)| *t == a.trigger).map(|(_, s)| *s).unwrap_or("the event happens")))),
        |ctx, a| {
            if !intake::known_trigger(&a.trigger) {
                return Err(DevdError::invalid(format!("`{}` is not an event devd watches; use one of {}", a.trigger, intake::TRIGGERS.iter().map(|(t, _)| *t).collect::<Vec<_>>().join(", "))));
            }
            let project = std::path::Path::new(&a.project).canonicalize().map(|d| d.display().to_string()).unwrap_or_else(|_| a.project.clone());
            if !std::path::Path::new(&project).is_dir() {
                return Err(DevdError::not_found(format!("{} is not a folder", a.project)));
            }
            let name = a.name.trim().to_ascii_lowercase();
            if name.is_empty() || name.len() > 40 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(DevdError::invalid("a rule name is 1-40 letters, digits, - or _"));
            }
            if a.goal.trim().chars().count() < 10 {
                return Err(DevdError::invalid("say what the run should do"));
            }
            let role = a.role.clone().unwrap_or_else(|| "implementer".into());
            if roles::get(&role).is_none() {
                return Err(DevdError::invalid(format!("`{role}` is not a member of the team")));
            }
            // A new rule observes unless the person said otherwise, and never
            // starts work in production without a plan approval, which is the
            // same rule the schedule path uses.
            let mode = a.mode.clone().unwrap_or_else(|| "observe".into());
            if !matches!(mode.as_str(), "observe" | "auto") {
                return Err(DevdError::invalid("mode is `observe` or `auto`"));
            }
            // The condition must be stored as an object. A shell that keeps the
            // braces, or a caller that sends the JSON as a string, would
            // otherwise store text that every later reader has to guess at.
            let mut condition = match a.condition.clone().unwrap_or_else(|| json!({})) {
                Value::String(text) => serde_json::from_str::<Value>(&text).map_err(|e| {
                    DevdError::invalid(format!("the condition is not a JSON object: {e}"))
                })?,
                other => other,
            };
            if !condition.is_object() {
                return Err(DevdError::invalid(
                    "the condition is an object, for example {\"min_failures\": 3, \"cooldown_minutes\": 60}",
                ));
            }
            if condition.get("min_failures").is_none() {
                condition["min_failures"] = json!(1);
            }
            if condition.get("min_failures").and_then(Value::as_i64).is_none() {
                return Err(DevdError::invalid("min_failures is a whole number"));
            }
            if condition.get("cooldown_minutes").is_some_and(|v| v.as_i64().is_none()) {
                return Err(DevdError::invalid("cooldown_minutes is a whole number"));
            }
            let existing = ctx.store.list_intake_rules(Some(&project))?.into_iter().find(|r| r.name == name);
            let rule = IntakeRule {
                project: project.clone(),
                name: name.clone(),
                trigger: a.trigger.clone(),
                condition,
                goal: a.goal.trim().to_string(),
                template: a.template.clone().filter(|t| !t.trim().is_empty()),
                role,
                mode: mode.clone(),
                countdown: existing.as_ref().map(|r| r.countdown).unwrap_or(0),
                fired_at: existing.as_ref().and_then(|r| r.fired_at.clone()),
                enabled: true,
            };
            ctx.store.upsert_intake_rule(&rule)?;
            Ok(Outcome::with_data(
                if mode == "auto" { "saved; it will start a run, which still waits for your plan approval" } else { "saved in observe mode: it records what it would have done" },
                rule_json(&rule),
            ))
        },
    )?;

    reg.define::<RuleRef>("intake.delete", "Delete a rule")
    .description("Removes one rule. The runs it already started stay. Human only.")
    .group("intake")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .human_only()
    .dry_run()
    .handlers(
        |_, a| simple("intake.delete", format!("delete rule {}", a.name)),
        |ctx, a| {
            if !ctx.store.delete_intake_rule(&a.project, &a.name)? {
                return Err(DevdError::not_found(format!("no rule called `{}` in {}", a.name, a.project)));
            }
            Ok(Outcome::ok(format!("deleted rule {}", a.name)))
        },
    )?;

    reg.define::<NoAutomationArgs>("automation.list", "List incoming hooks")
        .description("Every incoming hook: its name, event, filter, action and whether it is enabled. It does not reveal the token; a person can rotate one with automation.set. Read-only.")
        .group("automation")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .dry_run()
        .idempotent()
        .handlers(
            |_, _| simple("automation.list", "list incoming hooks"),
            |ctx, _| {
                let rows: Vec<Value> = ctx.store.list_automations()?.into_iter().map(|a| json!({
                    "name": a.name, "event": a.event, "filter": a.filter, "action": a.action,
                    "enabled": a.enabled, "created_at": a.created_at, "last_at": a.last_at,
                    "last_error": a.last_error
                })).collect();
                Ok(Outcome::with_data(format!("{} hook(s)", rows.len()), json!(rows)))
            },
        )?;

    reg.define::<AutomationSet>("automation.set", "Create or change an incoming hook")
        .description("Stores one rule for an incoming HTTP hook at /hooks/task/<token>. An event is matched against the filter, then the action starts a team run, messages the assistant, or posts an Inbox item. The token is returned once here. Human only; a hook is a caller with no person behind it, so it can only do what the action names.")
        .group("automation")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .human_only()
        .dry_run()
        .handlers(
            |_, a| simple("automation.set", format!("store the hook `{}`", a.name)),
            |ctx, a| {
                let name = a.name.trim().to_ascii_lowercase();
                if name.is_empty() || name.len() > 40 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    return Err(DevdError::invalid("a hook name is 1-40 letters, digits, - or _"));
                }
                let event = a.event.trim();
                if event.is_empty() || event.len() > 80 {
                    return Err(DevdError::invalid("say which event this hook accepts"));
                }
                let action = match &a.action {
                    Value::String(text) => serde_json::from_str::<Value>(text)
                        .map_err(|e| DevdError::invalid(format!("the action is not JSON: {e}")))?,
                    other => other.clone(),
                };
                let kind = action["type"].as_str().unwrap_or("");
                match kind {
                    "team" => {
                        if action["goal"].as_str().unwrap_or("").trim().chars().count() < 10 {
                            return Err(DevdError::invalid("a team action needs a goal of at least 10 characters"));
                        }
                    }
                    "chat" => {
                        if action["message"].as_str().unwrap_or("").trim().is_empty() {
                            return Err(DevdError::invalid("a chat action needs a message"));
                        }
                    }
                    "inbox" => {
                        if action["subject"].as_str().unwrap_or("").trim().is_empty() {
                            return Err(DevdError::invalid("an inbox action needs a subject"));
                        }
                    }
                    _ => return Err(DevdError::invalid("the action type is team, chat or inbox")),
                }
                let filter = match a.filter.clone().unwrap_or_else(|| json!({})) {
                    Value::String(text) => serde_json::from_str::<Value>(&text)
                        .map_err(|e| DevdError::invalid(format!("the filter is not JSON: {e}")))?,
                    other => other,
                };
                if !filter.is_object() {
                    return Err(DevdError::invalid("the filter is an object"));
                }
                // Editing a hook (turning it off, changing its goal) must not move its address
                // from under the sender; a new address is only made when asked for.
                let kept = if a.rotate.unwrap_or(false) {
                    None
                } else {
                    ctx.store.list_automations()?.into_iter().find(|x| x.name == name).map(|x| x.token)
                };
                let token = a.token.clone().filter(|t| t.trim().len() >= 16).or(kept).unwrap_or_else(|| util::new_id());
                if token.len() < 16 {
                    return Err(DevdError::invalid("a token is at least 16 characters"));
                }
                ctx.store.put_automation(&name, &token, event, &filter, &action, a.enabled.unwrap_or(true))?;
                Ok(Outcome::with_data(
                    "saved; the hook address is /hooks/task/<token>",
                    json!({ "name": name, "event": event, "token": token, "url": format!("/hooks/task/{token}") }),
                ))
            },
        )?;

    reg.define::<AutomationRef>("automation.delete", "Delete an incoming hook")
        .description("Removes one hook. Calls already accepted are not touched. Human only.")
        .group("automation")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .human_only()
        .dry_run()
        .handlers(
            |_, a| simple("automation.delete", format!("delete the hook `{}`", a.name)),
            |ctx, a| {
                if !ctx.store.delete_automation(a.name.trim())? {
                    return Err(DevdError::not_found(format!("no hook called `{}`", a.name)));
                }
                Ok(Outcome::ok("deleted"))
            },
        )?;

    Ok(())
}
