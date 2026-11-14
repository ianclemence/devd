//! Runs: what the team is doing, and what it did.
//!
//! The list on the left is grouped by project and can be narrowed to the runs that need you, the ones working and the
//! ones done. A run's page reads from the top in the order a person asks: what is it and what has it cost; is anything
//! wrong; what is happening right now; is it meeting what it was asked to meet; who is doing what; where the time went.
//! A finished run leads with its result, and the process comes after. Choosing an agent, in the chain or the list, opens
//! what it was asked, what it changed, what it checked and what it cost beside the page.
//!
//! Every value comes from the run as devd reports it (`team.get`, `team.timeline`, the conversations' own usage); the
//! sentences come from the same rule the assistant answers with.

use devd_app_core::nav::Route;
use devd_app_core::status;
use devd_app_core::sync::{Mutation, Patch, QueryKey};
use gpui::{AnyElement, ClickEvent, Context, Div, FontWeight, Hsla, div, prelude::*, px};
use serde_json::{Value, json};

use crate::shell::Shell;
use crate::state::{devd, q};
use crate::theme::{Tone, mono, size, theme};
use crate::ui::{self, Button, ButtonKind, Clip, eyebrow, icon, tint};

/// Which runs the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    /// Every run.
    #[default]
    All,
    /// Paused, failed, or waiting for the plan to be approved.
    NeedsYou,
    /// Planning or running.
    Working,
    /// Done or cancelled.
    Done,
}

fn needs_you(state: &str) -> bool {
    matches!(state, "paused" | "failed" | "awaiting_plan")
}

fn working(state: &str) -> bool {
    matches!(state, "planning" | "running")
}

fn over(state: &str) -> bool {
    matches!(state, "done" | "failed" | "cancelled")
}

impl Filter {
    fn admits(self, state: &str) -> bool {
        match self {
            Filter::All => true,
            Filter::NeedsYou => needs_you(state),
            Filter::Working => working(state),
            Filter::Done => matches!(state, "done" | "cancelled"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::NeedsYou => "Needs you",
            Filter::Working => "Working",
            Filter::Done => "Done",
        }
    }
}

/// Where a run stands, as one small glyph: the same glyphs the chain uses for its tasks.
fn glyph(id: impl Into<gpui::ElementId>, state: &str, cx: &Context<Shell>) -> AnyElement {
    let t = theme(cx);
    let frame = || {
        div()
            .size(px(14.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
    };
    match state {
        s if working(s) => frame()
            .child(ui::live_dot(id, Tone::Accent, cx))
            .into_any_element(),
        "done" => frame().child(icon("check", 13.0, t.ok)).into_any_element(),
        s if needs_you(s) => frame()
            .child(icon("alert", 13.0, t.warn))
            .into_any_element(),
        _ => frame()
            .child(
                div()
                    .size(px(9.0))
                    .rounded_full()
                    .border_1()
                    .border_color(t.faint),
            )
            .into_any_element(),
    }
}

/// A run's state in a word or two, as a person would say it.
fn state_words(state: &str) -> &'static str {
    match state {
        "planning" => "Planning",
        "awaiting_plan" => "Plan to approve",
        "running" => "Working",
        "paused" => "Paused",
        "done" => "Done",
        "failed" => "Failed",
        "cancelled" => "Cancelled",
        _ => "Waiting",
    }
}

/// A task's state in a word, the way the chain says it: work accepted by hand reads as accepted, not skipped.
fn task_word(state: &str) -> String {
    match state {
        "skipped" => "Accepted".to_string(),
        "pending" => "Waiting".to_string(),
        other => status::of(other).label.clone(),
    }
}

/// Text of several paragraphs, each wrapping on its own, cut at a whole word once it passes `limit` characters. The rest is
/// in the agent's conversation.
fn paragraphs(text: &str, limit: usize, colour: Hsla) -> Div {
    let mut out = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .text_size(px(size::TEXT_SM))
        .line_height(px(20.0))
        .text_color(colour);
    let mut used = 0;
    for para in text
        .split(
            "
",
        )
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        if used >= limit {
            break;
        }
        let room = limit - used;
        let shown = if para.chars().count() > room {
            let cut: String = para.chars().take(room).collect();
            let cut = cut
                .rsplit_once(' ')
                .map(|(head, _)| head.to_string())
                .unwrap_or(cut);
            format!("{cut} …")
        } else {
            para.to_string()
        };
        used += para.chars().count();
        out = out.child(div().w_full().min_w_0().child(shown));
    }
    out
}

/// The screen.
pub fn render(shell: &mut Shell, selected: Option<String>, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let detail: AnyElement = match selected {
        None => div()
            .flex_1()
            .child(ui::empty(
                "runs",
                "Choose a run",
                "What it is doing, what it has cost, and who is working on what.",
                cx,
            ))
            .into_any_element(),
        Some(id) => run_page(shell, &id, cx),
    };
    // A screen of work: a solid ground, so a chain and a timeline are read against nothing but themselves.
    div()
        .size_full()
        .flex()
        .bg(t.bg)
        .child(list(shell, cx))
        .child(detail)
        .into_any_element()
}

/// The runs, grouped by project, with the filter above them.
fn list(shell: &mut Shell, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let snap = devd(cx).sync.watch(&q::runs());
    let runs: Vec<Value> = snap.list("").to_vec();
    if runs
        .iter()
        .any(|r| !over(r["state"].as_str().unwrap_or_default()))
    {
        devd(cx).run_seen.set(true);
    }
    let filter = shell.runs_filter;
    let selected = match shell.history.current() {
        Route::Runs(Some(id)) => Some(id.clone()),
        _ => None,
    };

    // The filter: four words, the chosen one in the text colour with a hairline under it.
    let mut tabs = div()
        .flex()
        .items_center()
        .gap(px(14.0))
        .px(px(16.0))
        .h(px(36.0))
        .flex_none()
        .border_b_1()
        .border_color(t.line);
    for (n, f) in [Filter::All, Filter::NeedsYou, Filter::Working, Filter::Done]
        .into_iter()
        .enumerate()
    {
        let on = f == filter;
        let count = runs
            .iter()
            .filter(|r| f.admits(r["state"].as_str().unwrap_or_default()))
            .count();
        let hover = t.text;
        tabs = tabs.child(
            div()
                .id(ui::nid("runs.filter", n))
                .h_full()
                .flex()
                .items_center()
                .gap(px(5.0))
                .cursor_pointer()
                .border_b_1()
                .border_color(if on { t.text } else { tint(t.text, 0.0) })
                .text_size(px(size::TEXT_SM))
                .text_color(if on { t.text } else { t.muted })
                .hover(move |s| s.text_color(hover))
                .child(f.label())
                .children(
                    (count > 0 && f != Filter::All)
                        .then(|| div().text_color(t.faint).child(count.to_string())),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.runs_filter = f;
                    cx.notify();
                })),
        );
    }

    let mut column = div().flex().flex_col().gap(px(1.0)).p(px(8.0));
    if let Some(shown) = (snap.is_first_load()).then(|| {
        div()
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .children((0..5).map(|n| ui::skeleton(ui::nid("runs.sk", n), 200.0, cx)))
    }) {
        column = column.child(shown);
    }
    let shown: Vec<&Value> = runs
        .iter()
        .filter(|r| filter.admits(r["state"].as_str().unwrap_or_default()))
        .collect();
    if !snap.is_first_load() && shown.is_empty() {
        let (title, detail) = match filter {
            Filter::All => (
                "No runs yet",
                "Give the team a goal from a conversation, and its run appears here.",
            ),
            Filter::NeedsYou => (
                "Nothing needs you",
                "A run that pauses, fails or waits for its plan to be approved shows here.",
            ),
            Filter::Working => (
                "Nothing is working",
                "Runs that are planning or working show here.",
            ),
            Filter::Done => (
                "Nothing finished yet",
                "Finished and cancelled runs show here.",
            ),
        };
        column = column.child(
            div()
                .px(px(12.0))
                .py(px(16.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.muted)
                        .child(detail),
                ),
        );
    }
    let mut last_project: Option<String> = None;
    for run in shown {
        let id = run["id"].as_str().unwrap_or_default().to_string();
        let state = run["state"].as_str().unwrap_or_default().to_string();
        let project = run["project"]
            .as_str()
            .map(|p| ui::leaf(p).to_string())
            .unwrap_or_else(|| "No project".to_string());
        if last_project.as_ref() != Some(&project) {
            column = column.child(
                div()
                    .px(px(10.0))
                    .pt(px(12.0))
                    .pb(px(4.0))
                    .child(eyebrow(project.to_uppercase(), cx)),
            );
            last_project = Some(project);
        }
        let (done, total, active) = (
            run["tasks_done"].as_u64().unwrap_or(0),
            run["tasks_total"].as_u64().unwrap_or(0),
            run["tasks_active"].as_u64().unwrap_or(0),
        );
        let mut facts = vec![state_words(&state).to_string()];
        if total > 0 {
            facts.push(format!("{done} of {total}"));
        }
        if active > 0 && working(&state) {
            facts.push(format!("{active} working"));
        }
        facts.push(ui::ago(run["updated_at"].as_str().unwrap_or_default()));
        let (go, doomed, doomed_title) = (
            id.clone(),
            id.clone(),
            run["title"].as_str().unwrap_or("this run").to_string(),
        );
        column = column.child(
            ui::row(
                gpui::ElementId::Name(format!("run.{id}").into()),
                selected.as_deref() == Some(id.as_str()),
                cx,
            )
            .group("run.row")
            .h(px(48.0))
            .child(glyph(
                gpui::ElementId::Name(format!("run.glyph.{id}").into()),
                &state,
                cx,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(size::TEXT)).child(Clip::new(
                        title_of(run),
                    )))
                    .child(
                        div()
                            .text_size(px(size::TEXT_XS))
                            .text_color(if needs_you(&state) { t.warn } else { t.faint })
                            .child(facts.join(" · ")),
                    ),
            )
            .children(over(&state).then(|| {
                ui::row_action(
                    gpui::ElementId::Name(format!("run.delete.{id}").into()),
                    "run.row",
                    "trash",
                    "Delete run",
                    true,
                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.delete_run(doomed.clone(), doomed_title.clone(), window, cx)
                    }),
                    cx,
                )
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.go(Route::Runs(Some(go.clone())), cx)
            })),
        );
    }
    let clear = div().flex_none().p(px(8.0)).border_t_1().border_color(t.line).child(
        Button::new("runs.clear", "Clear finished").icon("trash").kind(ButtonKind::Ghost).on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.confirm(
                "Delete every finished run?".to_string(),
                "Runs that are done, failed or cancelled are removed, with their tasks, logs and the conversations of the agents that worked on them. Runs still going are kept, and so are the files in your projects. This cannot be undone.".to_string(),
                "Delete finished",
                true,
                Mutation::new("team.prune", json!({})).invalidating("team"),
                window,
                cx,
            )
        })),
    );
    div()
        .w(px(size::SIDEBAR + 40.0))
        .h_full()
        .flex()
        .flex_col()
        .flex_none()
        .border_r_1()
        .border_color(t.line)
        .child(
            div()
                .h(px(44.0))
                .px(px(16.0))
                .flex()
                .flex_none()
                .items_center()
                .font_weight(FontWeight::MEDIUM)
                .child("Runs"),
        )
        .child(tabs)
        .child(
            div()
                .id("runs.list")
                .flex_1()
                .overflow_y_scroll()
                .child(column),
        )
        .child(clear)
        .into_any_element()
}

/// One run.
fn run_page(shell: &mut Shell, id: &str, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let snap = devd(cx).sync.watch(&q::run(id));
    let Some(data) = snap.data.clone() else {
        let body = match &snap.error {
            Some(failure) if failure.code == "not_found" => ui::empty(
                "runs",
                "This run is gone",
                "It was deleted, here or somewhere else.",
                cx,
            ),
            Some(failure) => ui::empty(
                "alert",
                "This run could not be read",
                failure.message.clone(),
                cx,
            ),
            None => {
                div().p(px(32.0)).flex().flex_col().gap(px(16.0)).children(
                    (0..5).map(|n| {
                        ui::skeleton(ui::nid("run.skeleton", n), 240.0 + n as f32 * 50.0, cx)
                    }),
                )
            }
        };
        return div().flex_1().child(body).into_any_element();
    };
    let run = if data["run"].is_object() {
        data["run"].clone()
    } else {
        (*data).clone()
    };
    let tasks: Vec<Value> = data["tasks"].as_array().cloned().unwrap_or_default();
    let state = run["state"].as_str().unwrap_or_default().to_string();
    // The agent shown beside the page belongs to this run; choosing another run closes it.
    if shell.run_focus.as_ref().is_some_and(|(r, _)| r != id) {
        shell.run_focus = None;
    }
    let focus = shell.run_focus.as_ref().map(|(_, k)| k.clone());

    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(28.0))
        .child(heading(id, &run, &data, &tasks, cx));
    let attended = attention(id, &run, &data, cx);
    // A paused run's banner already says what is holding it; the sentence about where it stands would say it again.
    let said = attended.is_some() && state == "paused";
    if let Some(line) = attended {
        column = column.child(line);
    }
    if over(&state) {
        if let Some(result) = result(&run, &tasks, cx) {
            column = column.child(result);
        }
    } else if !said {
        column = column.child(now(&state, &tasks, cx));
    }
    if let Some(criteria) = criteria(&data, &tasks, cx) {
        column = column.child(criteria);
    }
    if !tasks.is_empty() {
        let chain = ui::flow::graph_selectable("run.chain", &tasks, focus.as_deref(), id, cx);
        column = column.child(section("THE TEAM", None, chain, cx));
        column = column.child(timeline(id, cx));
        column = column.child(section(
            "TASKS",
            None,
            task_list(id, &tasks, focus.as_deref(), cx),
            cx,
        ));
    }
    column = column.child(section(
        "GOAL",
        None,
        div()
            .text_size(px(size::TEXT))
            .line_height(px(21.0))
            .text_color(t.muted)
            .child(run["goal"].as_str().unwrap_or_default().to_string())
            .into_any_element(),
        cx,
    ));

    let page = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(ui::page("run.detail", 900.0, column));
    let mut row = div().flex_1().min_w_0().h_full().flex().child(page);
    if let Some(task) = focus.and_then(|key| {
        tasks
            .iter()
            .find(|t| t["key"].as_str() == Some(key.as_str()))
    }) {
        row = row.child(agent_panel(id, task, &data, cx));
    }
    row.into_any_element()
}

/// A labelled part of the page.
fn section(
    label: &'static str,
    aside: Option<String>,
    body: AnyElement,
    cx: &Context<Shell>,
) -> AnyElement {
    let t = theme(cx);
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .child(eyebrow(label, cx))
                .children(aside.map(|a| {
                    div()
                        .text_size(px(size::TEXT_XS))
                        .text_color(t.faint)
                        .child(a)
                })),
        )
        .child(body)
        .into_any_element()
}

/// The heading: the title, one line of facts with what the run has spent, and its actions.
fn heading(
    id: &str,
    run: &Value,
    data: &Value,
    tasks: &[Value],
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    let state = run["state"].as_str().unwrap_or_default().to_string();
    let mut facts = vec![state_words(&state).to_string()];
    if let Some(project) = run["project"].as_str() {
        facts.push(ui::leaf(project).to_string());
    }
    if !tasks.is_empty() {
        facts.extend(ui::flow::summary(tasks).split(" · ").map(str::to_string));
    }
    let usage = &data["usage"];
    let budget = &data["budget"];
    if let Some(cost) = usage["cost_usd"].as_f64() {
        facts.push(match budget["usd"].as_f64() {
            Some(limit) => format!("${cost:.2} of ${limit:.2}"),
            None => format!("${cost:.2} spent"),
        });
    }
    if let Some(turns) = usage["turns"].as_u64().filter(|n| *n > 0) {
        facts.push(match budget["turns"].as_u64() {
            Some(limit) => format!("{turns} of {limit} turns"),
            None => format!("{turns} turns"),
        });
    }
    facts.push(format!(
        "started {}",
        ui::ago(run["created_at"].as_str().unwrap_or_default())
    ));

    let mut actions = div().flex().items_center().gap(px(4.0));
    let run_id = id.to_string();
    match state.as_str() {
        // A run paused for a reason says so in a banner, and Resume is there, beside the reason; once is enough.
        "paused" if run["error"].as_str().is_none_or(str::is_empty) => {
            let resume = run_id.clone();
            actions = actions.child(
                Button::new("run.resume", "Resume")
                    .icon("play")
                    .kind(ButtonKind::Primary)
                    .on_click(move |_, _, cx| {
                        devd(cx).sync.mutate(
                            Mutation::new("team.resume", json!({ "run": resume })).optimistic(
                                Patch::set(q::run(&resume), "/run/state", json!("running")),
                            ),
                        );
                    }),
            );
        }
        "planning" | "running" => {
            let pause = run_id.clone();
            actions = actions.child(
                Button::new("run.pause", "Pause")
                    .icon("pause")
                    .kind(ButtonKind::Ghost)
                    .on_click(move |_, _, cx| {
                        devd(cx).sync.mutate(
                            Mutation::new("team.pause", json!({ "run": pause }))
                                .invalidating("team"),
                        );
                    }),
            );
        }
        _ => {}
    }
    if !over(&state) {
        let cancel = run_id.clone();
        actions = actions.child(div().id("run.cancel.wrap").tooltip(ui::tip("Cancel the run")).child(Button::icon_only("run.cancel", "stop").on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.confirm("Cancel this run?".to_string(), "The agents stop where they are. What they already wrote stays in their working copies, and the run can be deleted afterwards.".to_string(), "Cancel run", true, Mutation::new("team.cancel", json!({ "run": cancel })).invalidating("team"), window, cx)
        }))));
    } else {
        let (doomed, title) = (
            run_id.clone(),
            run["title"].as_str().unwrap_or("this run").to_string(),
        );
        actions = actions.child(
            div()
                .id("run.delete.wrap")
                .tooltip(ui::tip("Delete the run"))
                .child(
                    Button::icon_only("run.delete", "trash").on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            this.delete_run(doomed.clone(), title.clone(), window, cx)
                        },
                    )),
                ),
        );
    }
    if let Some(thread) = run["thread_id"].as_str().map(str::to_string) {
        actions = actions.child(
            div()
                .id("run.thread.wrap")
                .tooltip(ui::tip("Open the conversation that asked for it"))
                .child(
                    Button::icon_only("run.thread", "assistant").on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.go(Route::Assistant(Some(thread.clone())), cx)
                        },
                    )),
                ),
        );
    }
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap(px(20.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(px(size::TEXT_XL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_height(px(30.0))
                        .line_clamp(2)
                        .text_color(t.text)
                        .child(title_of(run)),
                )
                .child(
                    div()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.muted)
                        .child(facts.join("  ·  ")),
                ),
        )
        .child(actions)
        .into_any_element()
}

/// What needs the person's attention, on one quiet line with the action that resolves it.
fn attention(id: &str, run: &Value, data: &Value, cx: &mut Context<Shell>) -> Option<AnyElement> {
    let t = theme(cx).clone();
    let line = |tone: Hsla, text: String, action: Option<AnyElement>| {
        let tone = if tone == t.danger { Tone::Danger } else { Tone::Warn };
        let mut banner = ui::Banner::new(tone).text(text);
        if let Some(action) = action {
            banner = banner.action(action);
        }
        banner.render(cx).into_any_element()
    };
    if data["driver"] == "none" {
        let carry = id.to_string();
        let button = Button::new("run.drive", "Carry it on")
            .kind(ButtonKind::Primary)
            .on_click(move |_, _, cx| {
                devd(cx).sync.mutate(
                    Mutation::new("team.drive", json!({ "run": carry })).invalidating("team"),
                );
            });
        return Some(line(t.warn, "Nothing is carrying this run. Its work is kept; carry it on to continue from where it stopped.".to_string(), Some(button.into_any_element())));
    }
    let error = run["error"].as_str().filter(|e| !e.is_empty())?;
    // A paused run is answered where it says why: resuming gives the task that stopped it another go.
    let resume = (run["state"] == "paused").then(|| {
        let again = id.to_string();
        Button::new("run.resume.banner", "Resume")
            .icon("play")
            .kind(ButtonKind::Primary)
            .on_click(move |_, _, cx| {
                devd(cx).sync.mutate(
                    Mutation::new("team.resume", json!({ "run": again }))
                        .optimistic(Patch::set(q::run(&again), "/run/state", json!("running")))
                        .invalidating("team"),
                );
            })
            .into_any_element()
    });
    Some(line(
        if run["state"] == "failed" {
            t.danger
        } else {
            t.warn
        },
        error.to_string(),
        resume,
    ))
}

/// What is happening now, as one sentence, level with its mark.
fn now(state: &str, tasks: &[Value], cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    div()
        .flex()
        .items_start()
        .gap(px(10.0))
        .child(
            div()
                .h(px(24.0))
                .flex_none()
                .flex()
                .items_center()
                .child(glyph("run.now", state, cx)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(size::TEXT_MD))
                .line_height(px(24.0))
                .text_color(t.text)
                .child(ui::flow::status_line(state, tasks)),
        )
        .into_any_element()
}

/// A finished run's result, first: its report and its verdict.
fn result(run: &Value, tasks: &[Value], cx: &mut Context<Shell>) -> Option<AnyElement> {
    let t = theme(cx).clone();
    let summary = run["summary"].as_str().filter(|s| !s.is_empty());
    let verdict = run["verdict"].as_str();
    if summary.is_none() && verdict.is_none() {
        return None;
    }
    let mut body = div().flex().flex_col().gap(px(12.0));
    if let Some(verdict) = verdict {
        let (tone, words) = match verdict {
            "approve" => (t.ok, "Approved by the final check"),
            "accepted_by_hand" => (t.muted, "Accepted by a person"),
            "changes_needed" => (t.warn, "The final check asked for changes"),
            _ => (t.warn, "The final check could not approve it"),
        };
        let checks = tasks
            .iter()
            .flat_map(|t| t["evidence"].as_array().cloned().unwrap_or_default())
            .filter(|e| e["kind"] == "verify")
            .collect::<Vec<_>>();
        let passed = checks.iter().filter(|e| e["ok"] == true).count();
        let mut line = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(size::TEXT_SM))
            .child(icon(
                if verdict == "approve" {
                    "check"
                } else {
                    "alert"
                },
                13.0,
                tone,
            ))
            .child(div().text_color(t.text).child(words));
        if !checks.is_empty() {
            line = line.child(
                div()
                    .text_color(t.faint)
                    .child(format!("·  {passed} of {} checks passed", checks.len())),
            );
        }
        if let Some(commit) = run["reviewed_commit"].as_str() {
            line = line.child(
                div()
                    .font_family(mono())
                    .text_size(px(size::TEXT_XS))
                    .text_color(t.faint)
                    .child(format!("·  {}", &commit[..commit.len().min(8)])),
            );
        }
        body = body.child(line);
    }
    if let Some(summary) = summary {
        let blocks = devd_app_core::markdown::parse(summary);
        body = body.child(
            div()
                .text_size(px(size::TEXT_MD))
                .line_height(px(22.0))
                .child(ui::markdown::document("run.summary", &blocks, cx)),
        );
    }
    Some(section("RESULT", None, body.into_any_element(), cx))
}

/// The definition of done, as a checklist: each line with what the final check found, or that it has not been checked yet.
fn criteria(data: &Value, tasks: &[Value], cx: &mut Context<Shell>) -> Option<AnyElement> {
    let t = theme(cx).clone();
    let lines: Vec<String> = data["done_when"]
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if lines.is_empty() {
        return None;
    }
    // What the gatekeepers said about each criterion, most recent last, matched by the words of the criterion.
    let judged: Vec<Value> = tasks
        .iter()
        .flat_map(|t| {
            t["verdict_data"]["criteria"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    let status_of = |line: &str| -> Option<String> {
        let probe: String = line.chars().take(40).collect::<String>().to_lowercase();
        judged
            .iter()
            .rev()
            .find(|c| {
                c["text"].as_str().is_some_and(|text| {
                    text.to_lowercase().contains(&probe)
                        || line.to_lowercase().contains(&text.to_lowercase())
                })
            })
            .and_then(|c| c["status"].as_str().map(str::to_string))
    };
    let mut met = 0;
    let mut list = div().flex().flex_col();
    for (n, line) in lines.iter().enumerate() {
        let status = status_of(line);
        let (mark, colour, note): (AnyElement, Hsla, &str) = match status.as_deref() {
            Some("passed" | "met" | "pass" | "ok") => {
                met += 1;
                (icon("check", 13.0, t.ok).into_any_element(), t.text, "")
            }
            Some("failed" | "not_met" | "fail") => (
                icon("alert", 13.0, t.danger).into_any_element(),
                t.text,
                "not met",
            ),
            _ => (
                div()
                    .size(px(9.0))
                    .rounded_full()
                    .border_1()
                    .border_color(t.faint)
                    .into_any_element(),
                t.muted,
                "not checked yet",
            ),
        };
        list = list.child(
            div()
                .id(ui::nid("criterion", n))
                .flex()
                .items_start()
                .gap(px(10.0))
                .py(px(6.0))
                .child(
                    div()
                        .h(px(20.0))
                        .w(px(14.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(mark),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(size::TEXT))
                        .line_height(px(20.0))
                        .text_color(colour)
                        .child(line.clone()),
                )
                .children((!note.is_empty()).then(|| {
                    div()
                        .flex_none()
                        .h(px(20.0))
                        .flex()
                        .items_center()
                        .text_size(px(size::TEXT_XS))
                        .text_color(t.faint)
                        .child(note)
                })),
        );
    }
    Some(section(
        "DONE WHEN",
        Some(format!("{met} of {} met", lines.len())),
        list.into_any_element(),
        cx,
    ))
}

/// Every task, one row each: what it is, who, and where it stands. Choosing one opens it beside the page.
fn task_list(
    run_id: &str,
    tasks: &[Value],
    focus: Option<&str>,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    let mut list = div().flex().flex_col();
    for (n, task) in tasks.iter().enumerate() {
        let key = task["key"].as_str().unwrap_or_default().to_string();
        let state = task["state"].as_str().unwrap_or_default().to_string();
        let said = status::of(&state);
        let tone: Tone = said.level.into();
        let on = focus == Some(key.as_str());
        let hover = t.hover;
        let (run, pick) = (run_id.to_string(), key.clone());
        list = list.child(
            div()
                .id(ui::nid("run.task", n))
                .h(px(40.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(12.0))
                .rounded(px(size::RADIUS))
                .cursor_pointer()
                .bg(if on { t.active } else { tint(t.active, 0.0) })
                .hover(move |s| s.bg(hover))
                .child(ui::flow::task_glyph(&state, n, cx))
                .child(
                    div()
                        .w(px(120.0))
                        .flex_none()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.muted)
                        .child(Clip::new(
                            task["role"]
                                .as_str()
                                .unwrap_or_default()
                                .replace(['-', '_'], " "),
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(size::TEXT))
                        .text_color(t.text)
                        .child(Clip::new(
                            task["title"].as_str().unwrap_or_default().to_string(),
                        )),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(size::TEXT_SM))
                        .text_color(if matches!(tone, Tone::Warn | Tone::Danger) {
                            t.tone(tone)
                        } else {
                            t.faint
                        })
                        .child(task_word(&state)),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.run_focus = if this
                        .run_focus
                        .as_ref()
                        .is_some_and(|(r, k)| r == &run && k == &pick)
                    {
                        None
                    } else {
                        Some((run.clone(), pick.clone()))
                    };
                    cx.notify();
                })),
        );
    }
    list.into_any_element()
}

/// Where each agent's time went: one line per task, spans of model turns, tool calls and checks along it, and what happened
/// to it marked where it happened.
fn timeline(run_id: &str, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let snap = devd(cx)
        .sync
        .watch(&QueryKey::new("team.timeline", json!({ "run": run_id })));
    let Some(data) = snap.data.clone() else {
        let body = match &snap.error {
            // A devd from before the timeline existed has nothing to give; the section is simply not shown.
            Some(_) => return div().into_any_element(),
            None => ui::skeleton("timeline.sk", 320.0, cx).into_any_element(),
        };
        return section("TIME", None, body, cx);
    };
    let tasks: Vec<Value> = data["tasks"].as_array().cloned().unwrap_or_default();
    // The scale runs to the last thing anyone did, not to now: a run paused for an afternoon would otherwise squeeze all
    // of its work into the first few pixels.
    let since = data["length"].as_f64().unwrap_or(0.0).max(1.0);
    let mut length: f64 = 1.0;
    for task in &tasks {
        for v in [task["start"].as_f64(), task["end"].as_f64()]
            .into_iter()
            .flatten()
        {
            length = length.max(v);
        }
        for span in task["spans"].as_array().into_iter().flatten() {
            length = length.max(span["end"].as_f64().unwrap_or(0.0));
        }
        for mark in task["marks"].as_array().into_iter().flatten() {
            length = length.max(mark["at"].as_f64().unwrap_or(0.0));
        }
    }
    let live = tasks.iter().any(|t| t["state"] == "running");
    if live {
        length = since;
    }
    const LABEL: f32 = 120.0;
    let x = |secs: f64| (secs / length).clamp(0.0, 1.0) as f32;
    let at = |secs: f64| gpui::relative(x(secs));
    let across = |a: f64, b: f64| gpui::relative((x(b) - x(a)).max(0.002));
    let mut rows = div().flex().flex_col().gap(px(6.0));
    for (n, task) in tasks.iter().enumerate() {
        let mut track = div().relative().flex_1().min_w_0().h(px(14.0));
        if task["start"].is_null() && task["spans"].as_array().is_none_or(|s| s.is_empty()) {
            track = track.child(
                div()
                    .absolute()
                    .top(px(-1.0))
                    .left_0()
                    .text_size(px(size::TEXT_XS))
                    .text_color(t.faint)
                    .child("not started"),
            );
        }
        // The task's own life, as a hairline from when it began to when it ended.
        if let Some(start) = task["start"].as_f64() {
            let end = task["end"].as_f64().unwrap_or(length);
            track = track.child(
                div()
                    .absolute()
                    .top(px(6.5))
                    .left(at(start))
                    .w(across(start, end))
                    .h(px(1.0))
                    .bg(t.line_strong),
            );
        }
        for span in task["spans"].as_array().cloned().unwrap_or_default() {
            let (start, end) = (
                span["start"].as_f64().unwrap_or(0.0),
                span["end"].as_f64().unwrap_or(0.0),
            );
            let colour = match span["kind"].as_str() {
                Some("check") => tint(t.ok, 0.85),
                Some("tool") => tint(t.text, 0.45),
                _ => tint(t.text, 0.18),
            };
            track = track.child(
                div()
                    .absolute()
                    .top(px(3.0))
                    .left(at(start))
                    .w(across(start, end))
                    .min_w(px(1.5))
                    .h(px(8.0))
                    .rounded(px(2.0))
                    .bg(colour),
            );
        }
        for mark in task["marks"].as_array().cloned().unwrap_or_default() {
            let colour = match mark["kind"].as_str() {
                Some("rejected" | "blocked") => t.danger,
                Some("provider" | "stalled" | "held" | "wrap_up") => t.warn,
                Some("accepted") => t.ok,
                _ => continue,
            };
            track = track.child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(at(mark["at"].as_f64().unwrap_or(0.0)))
                    .w(px(1.5))
                    .h(px(14.0))
                    .bg(colour),
            );
        }
        rows = rows.child(
            div()
                .id(ui::nid("timeline.row", n))
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(
                    div()
                        .w(px(LABEL))
                        .flex_none()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.muted)
                        .child(Clip::new(
                            task["key"].as_str().unwrap_or_default().to_string(),
                        )),
                )
                .child(track),
        );
    }
    let swatch = |colour: Hsla, words: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(div().w(px(10.0)).h(px(6.0)).rounded(px(2.0)).bg(colour))
            .child(words)
    };
    let tick = |colour: Hsla, words: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(div().w(px(1.5)).h(px(10.0)).bg(colour))
            .child(words)
    };
    let legend = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(16.0))
        .pl(px(LABEL + 12.0))
        .text_size(px(size::TEXT_XS))
        .text_color(t.faint)
        .child(swatch(tint(t.text, 0.18), "thinking"))
        .child(swatch(tint(t.text, 0.45), "tools"))
        .child(swatch(tint(t.ok, 0.85), "checks"))
        .child(tick(t.warn, "waiting or held"))
        .child(tick(t.danger, "sent back"));
    let words = |secs: f64| {
        let minutes = (secs / 60.0).round() as u64;
        if minutes >= 120 {
            format!("{} hours", minutes / 60)
        } else if minutes == 1 {
            "1 minute".to_string()
        } else {
            format!("{minutes} minutes")
        }
    };
    let span = if live || since - length < 120.0 {
        words(length)
    } else {
        format!(
            "{} of work, {} since it started",
            words(length),
            words(since)
        )
    };
    section(
        "TIME",
        Some(span),
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(rows)
            .child(legend)
            .into_any_element(),
        cx,
    )
}

/// One agent, beside the page: what it was asked, what it changed, what it checked, what it handed on and what it cost.
fn agent_panel(run_id: &str, task: &Value, data: &Value, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let key = task["key"].as_str().unwrap_or_default().to_string();
    let state = task["state"].as_str().unwrap_or_default().to_string();
    let block = |label: &'static str, body: Div| {
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(eyebrow(label, cx))
            .child(body.w_full().min_w_0())
    };
    let small = |text: String| {
        div()
            .text_size(px(size::TEXT_SM))
            .line_height(px(20.0))
            .text_color(t.text)
            .child(text)
    };
    let mut column = div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(22.0))
        .p(px(20.0));

    // Who, and where it stands.
    let close = div()
        .id("agent.close")
        .size(px(26.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(size::RADIUS))
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .child(icon("x", 13.0, t.muted))
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.run_focus = None;
            cx.notify();
        }));
    column = column.child(
        div()
            .flex()
            .items_start()
            .gap(px(10.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(task["title"].as_str().unwrap_or_default().to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(size::TEXT_SM))
                            .text_color(t.muted)
                            .child(format!(
                                "{} · {}",
                                task["role"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .replace(['-', '_'], " "),
                                task_word(&state)
                            )),
                    ),
            )
            .child(close),
    );

    // What it can do next, for the person.
    let mut actions = div().flex().flex_wrap().gap(px(6.0));
    if let Some(thread) = task["thread_id"].as_str().map(str::to_string) {
        actions = actions.child(
            Button::new("agent.open", "Open its conversation")
                .icon("assistant")
                .kind(ButtonKind::Secondary)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.go(Route::Assistant(Some(thread.clone())), cx)
                })),
        );
    }
    if matches!(state.as_str(), "blocked" | "failed" | "unverified") {
        let (retry, accept) = (
            task["id"].as_str().unwrap_or_default().to_string(),
            task["id"].as_str().unwrap_or_default().to_string(),
        );
        actions = actions.child(
            Button::new("agent.retry", "Try again")
                .icon("refresh")
                .kind(ButtonKind::Ghost)
                .on_click(move |_, _, cx| {
                    devd(cx).sync.mutate(
                        Mutation::new("team.retry", json!({ "task": retry })).invalidating("team"),
                    );
                }),
        );
        actions = actions.child(Button::new("agent.accept", "Accept as it is").kind(ButtonKind::Ghost).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.confirm("Accept this task without its proof?".to_string(), "The run carries on as if the task were finished, and its report says a person accepted it. Its work so far is committed.".to_string(), "Accept", false, Mutation::new("team.accept", json!({ "task": accept })).invalidating("team"), window, cx)
        })));
    }
    column = column.child(actions);
    if let Some(error) = task["error"].as_str().filter(|e| !e.is_empty()) {
        column = column.child(
            div()
                .text_size(px(size::TEXT_SM))
                .line_height(px(20.0))
                .text_color(t.warn)
                .child(error.to_string()),
        );
    }

    // What it was asked.
    column = column.child(block(
        "ITS BRIEF",
        paragraphs(
            task["instructions"].as_str().unwrap_or_default(),
            900,
            t.muted,
        ),
    ));
    let scope: Vec<String> = task["scope"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if !scope.is_empty() {
        column = column.child(block(
            "MAY CHANGE",
            div()
                .font_family(mono())
                .text_size(px(size::TEXT_XS))
                .text_color(t.muted)
                .child(scope.join("  ")),
        ));
    }

    // What it handed on: kept when the task was accepted; for one still working, what its evidence shows so far.
    let handoff = &data["handoffs"][key.as_str()];
    let evidence: Vec<Value> = task["evidence"].as_array().cloned().unwrap_or_default();
    let files: Vec<String> = handoff["files"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_else(|| {
            let mut seen = Vec::new();
            for e in evidence
                .iter()
                .filter(|e| e["kind"] == "change" && e["ok"] == true)
            {
                let path = e["summary"].as_str().unwrap_or_default().replace('\\', "/");
                let short = path
                    .rsplit('/')
                    .take(2)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("/");
                if !short.is_empty() && !seen.contains(&short) {
                    seen.push(short);
                }
            }
            seen
        });
    if !files.is_empty() {
        let mut list = div().flex().flex_col().gap(px(2.0));
        for f in files.iter().take(20) {
            list = list.child(
                div()
                    .font_family(mono())
                    .text_size(px(size::TEXT_XS))
                    .text_color(t.text)
                    .child(Clip::new(f.clone())),
            );
        }
        if files.len() > 20 {
            list = list.child(
                div()
                    .text_size(px(size::TEXT_XS))
                    .text_color(t.faint)
                    .child(format!("and {} more", files.len() - 20)),
            );
        }
        column = column.child(block("CHANGED", list));
    }
    let checks: Vec<&Value> = evidence.iter().filter(|e| e["kind"] == "verify").collect();
    if !checks.is_empty() {
        let mut list = div().flex().flex_col().gap(px(4.0));
        for c in checks.iter().rev().take(8) {
            let ok = c["ok"] == true;
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon(
                        if ok { "check" } else { "alert" },
                        12.0,
                        if ok { t.ok } else { t.danger },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(size::TEXT_SM))
                            .text_color(t.text)
                            .child(Clip::new(format!(
                                "{} {}",
                                c["capability"].as_str().unwrap_or_default(),
                                c["summary"].as_str().unwrap_or_default()
                            ))),
                    ),
            );
        }
        column = column.child(block("CHECKS", list));
    }
    for (label, field) in [("DECIDED", "decisions"), ("LEFT OPEN", "open_questions")] {
        let items: Vec<String> = handoff[field]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if !items.is_empty() {
            let mut list = div().flex().flex_col().gap(px(4.0));
            for item in items {
                list = list.child(small(format!("–  {item}")));
            }
            column = column.child(block(label, list));
        }
    }
    if let Some(summary) = handoff["summary"]
        .as_str()
        .or(task["result"].as_str())
        .filter(|s| !s.is_empty())
    {
        column = column.child(block("ITS REPORT", paragraphs(summary, 1200, t.muted)));
    }

    // What it cost: its conversation's own usage.
    if let Some(thread) = task["thread_id"].as_str() {
        let threads = devd(cx).sync.watch(&q::threads());
        if let Some(th) = threads
            .list("")
            .iter()
            .find(|th| th["id"].as_str() == Some(thread))
        {
            let tokens = th["input_tokens"].as_u64().unwrap_or(0)
                + th["output_tokens"].as_u64().unwrap_or(0);
            let cost = th["cost_usd"]
                .as_f64()
                .map(|c| format!("${c:.2}  ·  "))
                .unwrap_or_default();
            column = column.child(block(
                "COST",
                div()
                    .text_size(px(size::TEXT_SM))
                    .text_color(t.muted)
                    .child(format!(
                        "{cost}{} tokens  ·  {} attempts",
                        ui::compact(tokens),
                        task["attempts"].as_u64().unwrap_or(1)
                    )),
            ));
        }
    }
    let _ = run_id;
    div()
        .id("agent.panel")
        .w(px(380.0))
        .h_full()
        .flex_none()
        .overflow_x_hidden()
        .border_l_1()
        .border_color(t.line)
        .overflow_y_scroll()
        .child(column)
        .into_any_element()
}

/// A run's title as it is shown. Runs made before titles were cut at a word end half way through one ("(standard l");
/// such a title is the start of the goal, so it is cut again from the goal, at a word, with an ellipsis.
fn title_of(run: &Value) -> String {
    let title = run["title"].as_str().unwrap_or("Run").trim().to_string();
    let first = run["goal"].as_str().and_then(|g| g.lines().next()).unwrap_or_default().trim();
    if title.ends_with('…') || !first.starts_with(&title) || first.chars().count() <= title.chars().count() {
        return title;
    }
    let limit = title.chars().count().max(20);
    let cut: String = first.chars().take(limit.saturating_sub(1)).collect();
    let at = cut.rfind(char::is_whitespace).filter(|at| *at > limit / 2).unwrap_or(cut.len());
    let mut kept = &cut[..at];
    // A bracket opened and not closed in what is kept is dropped with what follows it.
    if kept.matches('(').count() > kept.matches(')').count()
        && let Some(open) = kept.rfind('(').filter(|open| *open > limit / 2)
    {
        kept = &kept[..open];
    }
    format!("{}…", kept.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '(' | '-')))
}
