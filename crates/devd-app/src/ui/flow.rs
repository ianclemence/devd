//! A team run drawn as a chain: each task a node, each dependency a line from the task that finishes first to the one
//! that waits for it.
//!
//! The arithmetic (which column, which row, where each line bends) is `devd_app_core::flow`. This draws it, and draws it
//! quietly. A node is a title and one line of facts, with a single glyph for where it stands: a check when finished, a
//! breathing dot while working, an empty ring while waiting, a warning when it has stopped. Lines are hairlines in the
//! colours of the page; the only colour in the picture is the hand-over happening now, and amber where work is held up.
//! Nothing is decorated: no stripes, no badges, no frames around the picture.

use std::cell::RefCell;
use std::collections::HashMap;

use devd_app_core::flow::{self, Flow, Metrics, Task};
use devd_app_core::nav::Route;
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, Hsla, PathBuilder, Pixels, Point, ScrollHandle,
    canvas, div, point, prelude::*, px,
};
use serde_json::Value;

use crate::shell::Shell;
use crate::state::devd;
use crate::theme::{Tone, size, theme};
use crate::ui::{self, Clip, tint};

/// The size of a node and the room between them.
const METRICS: Metrics = Metrics {
    node_w: 188.0,
    node_h: 46.0,
    gap_x: 44.0,
    gap_y: 10.0,
    margin: 4.0,
};

/// The tasks of a run, as the graph wants them.
pub fn tasks_of(tasks: &[Value]) -> Vec<Task> {
    tasks
        .iter()
        .map(|t| Task {
            key: t["key"].as_str().unwrap_or_default().to_string(),
            role: t["role"].as_str().unwrap_or_default().to_string(),
            title: t["title"].as_str().unwrap_or_default().to_string(),
            state: t["state"].as_str().unwrap_or("pending").to_string(),
            thread: t["thread_id"].as_str().map(str::to_string),
            depends_on: t["depends_on"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            quiet_secs: if t["state"] == "running" {
                t["last_step_at"]
                    .as_str()
                    .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                    .map(|at| {
                        (chrono::Utc::now() - at.with_timezone(&chrono::Utc))
                            .num_seconds()
                            .max(0) as u64
                    })
            } else {
                None
            },
            attempts: t["attempts"].as_u64().unwrap_or(1) as u32,
        })
        .collect()
}

/// One line, ready to paint.
struct Stroke {
    points: Vec<(f32, f32)>,
    color: Hsla,
    width: f32,
    dashed: bool,
}

thread_local! {
    /// What scrolls each graph sideways, and where it was last pointed.
    static VIEWS: RefCell<HashMap<&'static str, (ScrollHandle, String)>> = RefCell::new(HashMap::new());
}

/// The task the eye should be on: the first one working, else the first stopped, else the last finished.
fn frontier(tasks: &[Task]) -> Option<usize> {
    tasks
        .iter()
        .position(|t| t.state == "running")
        .or_else(|| tasks.iter().position(|t| flow::is_stuck(&t.state)))
        .or_else(|| tasks.iter().rposition(|t| flow::is_done(&t.state)))
}

/// What choosing a node does.
#[derive(Clone)]
enum Pick {
    /// Open the agent's conversation; the node of the conversation on screen is marked.
    Conversation(Option<String>),
    /// Show the agent beside the run's page; the chosen node is marked.
    Agent { run: String, chosen: Option<String> },
}

/// The graph of a run. `here` is the conversation on screen, whose node is marked; `id` keeps two graphs apart.
pub fn graph(
    id: &'static str,
    tasks: &[Value],
    here: Option<&str>,
    cx: &mut Context<Shell>,
) -> AnyElement {
    draw(id, tasks, Pick::Conversation(here.map(str::to_string)), cx)
}

/// The graph on a run's own page: choosing a node shows that agent beside the page, and `chosen` is marked.
pub fn graph_selectable(
    id: &'static str,
    tasks: &[Value],
    chosen: Option<&str>,
    run: &str,
    cx: &mut Context<Shell>,
) -> AnyElement {
    draw(
        id,
        tasks,
        Pick::Agent {
            run: run.to_string(),
            chosen: chosen.map(str::to_string),
        },
        cx,
    )
}

/// Where a task stands, as the chain shows it, for a list beside the chain.
pub fn task_glyph(state: &str, n: usize, cx: &mut Context<Shell>) -> AnyElement {
    let task = Task {
        key: String::new(),
        role: String::new(),
        title: String::new(),
        state: state.to_string(),
        thread: None,
        depends_on: Vec::new(),
        quiet_secs: None,
        attempts: 1,
    };
    glyph(&task, n + 10_000, cx)
}

fn draw(id: &'static str, tasks: &[Value], pick: Pick, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let tasks = tasks_of(tasks);
    if tasks.is_empty() {
        return div().into_any_element();
    }
    // A chain that is still changing is read again as it changes, whoever is driving the run.
    if tasks
        .iter()
        .any(|t| !flow::is_done(&t.state) && t.state != "cancelled" && t.state != "failed")
    {
        devd(cx).run_seen.set(true);
    }
    let layout = flow::layout(&tasks, METRICS);

    // Hairlines in the page's own colours; colour only where something is being handed on, or held up.
    let mut strokes: Vec<Stroke> = layout
        .edges
        .iter()
        .map(|e| {
            let (color, width, dashed) = match flow::flow(&tasks[e.from].state, &tasks[e.to].state)
            {
                Flow::Settled => (t.line_strong, 1.25, false),
                Flow::Live => (t.accent, 1.5, false),
                Flow::Held => (tint(t.warn, 0.8), 1.25, false),
                Flow::Waiting => (t.line, 1.25, true),
            };
            Stroke {
                points: e.points.clone(),
                color,
                width,
                dashed,
            }
        })
        .collect();
    // What is alive is painted last, so it lies over the quiet lines where they meet.
    strokes.sort_by(|a, b| {
        a.width
            .partial_cmp(&b.width)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let lines = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let at = |p: (f32, f32)| -> Point<Pixels> {
                point(bounds.origin.x + px(p.0), bounds.origin.y + px(p.1))
            };
            for stroke in &strokes {
                let mut path = PathBuilder::stroke(px(stroke.width));
                if stroke.dashed {
                    path = path.dash_array(&[px(3.0), px(4.0)]);
                }
                path.move_to(at(stroke.points[0]));
                for (n, pair) in stroke.points.windows(2).enumerate() {
                    let (a, b) = (pair[0], pair[1]);
                    // Between a node and a lane the line bends; along a lane it runs level.
                    if n % 2 == 0 {
                        let reach = (b.0 - a.0) * 0.5;
                        path.cubic_bezier_to(at(b), at((a.0 + reach, a.1)), at((b.0 - reach, b.1)));
                    } else {
                        path.line_to(at(b));
                    }
                }
                if let Ok(built) = path.build() {
                    window.paint_path(built, stroke.color);
                }
            }
        },
    )
    .absolute()
    .size_full();

    // A long chain is wider than its place. Bring the part that matters into view when it changes: the task working now,
    // or, in a conversation, the agent whose conversation this is. A person who scrolls away is left alone.
    let marked = |task: &Task| match &pick {
        Pick::Conversation(here) => here.is_some() && task.thread.as_deref() == here.as_deref(),
        Pick::Agent { chosen, .. } => chosen.as_deref() == Some(task.key.as_str()),
    };
    let focus = tasks.iter().position(&marked).or_else(|| frontier(&tasks));
    let handle = VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let (handle, last) = views
            .entry(id)
            .or_insert_with(|| (ScrollHandle::new(), String::new()));
        let signature = format!(
            "{:?}{}",
            focus,
            tasks
                .iter()
                .map(|t| t.state.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );
        if *last != signature {
            *last = signature;
            if let Some(at) = focus {
                let view = f32::from(handle.bounds().size.width);
                let view = if view > 0.0 { view } else { 640.0 };
                let x = layout.nodes[at].center.0 - view / 2.0;
                handle.set_offset(point(
                    px(-(x.clamp(0.0, (layout.width - view).max(0.0)))),
                    px(0.0),
                ));
            }
        }
        handle.clone()
    });

    let nodes: Vec<AnyElement> = layout
        .nodes
        .iter()
        .enumerate()
        .map(|(n, node)| task_node(&tasks[n], n, node.center, marked(&tasks[n]), &pick, cx))
        .collect();
    let inner = div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(layout.width))
        .h(px(layout.height))
        .child(lines)
        .children(nodes);
    div()
        .id(ui::nid(id, 1))
        .w_full()
        .overflow_x_scroll()
        .track_scroll(&handle)
        .child(inner)
        .into_any_element()
}

/// Where a task stands, as one small glyph.
fn glyph(task: &Task, n: usize, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let frame = || {
        div()
            .size(px(14.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
    };
    match task.state.as_str() {
        "running" => frame()
            .child(ui::live_dot(ui::nid("flow.live", n), Tone::Accent, cx))
            .into_any_element(),
        s if flow::is_done(s) => frame()
            .child(ui::icon("check", 13.0, t.ok))
            .into_any_element(),
        s if flow::is_stuck(s) => frame()
            .child(ui::icon("alert", 13.0, t.warn))
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

/// The facts under a task's title: its role and where it stands, in words.
fn facts(task: &Task) -> String {
    let role = task.role.replace(['-', '_'], " ");
    let state = match task.state.as_str() {
        "running" => match task.quiet_secs {
            Some(s) if s >= flow::QUIET_AFTER => format!("quiet for {}", flow::span(s)),
            Some(s) if s >= 20 => format!("working, {} ago", flow::span(s)),
            _ => "working".to_string(),
        },
        "pending" => "waiting".to_string(),
        "skipped" => "accepted".to_string(),
        other => other.replace('_', " "),
    };
    let attempt = if task.attempts > 1 && !flow::is_done(&task.state) {
        format!(" · try {}", task.attempts)
    } else {
        String::new()
    };
    format!("{role} · {state}{attempt}")
}

/// One task.
fn task_node(
    task: &Task,
    n: usize,
    center: (f32, f32),
    mine: bool,
    pick: &Pick,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    let stuck = flow::is_stuck(&task.state);
    let quiet = task.state == "running" && task.quiet_secs.is_some_and(|s| s >= flow::QUIET_AFTER);
    let hover = t.hover;
    let mut node = div()
        .id(ui::nid("flow.node", n))
        .absolute()
        .left(px(center.0 - METRICS.node_w / 2.0))
        .top(px(center.1 - METRICS.node_h / 2.0))
        .w(px(METRICS.node_w))
        .h(px(METRICS.node_h))
        .flex()
        .items_center()
        .gap(px(9.0))
        .px(px(10.0))
        .rounded(px(size::RADIUS_L))
        .border_1()
        .border_color(if mine { tint(t.accent, 0.7) } else { t.line })
        .bg(t.surface)
        .child(glyph(task, n, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(size::TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text)
                        .child(Clip::new(task.title.clone())),
                )
                .child(
                    div()
                        .text_size(px(size::TEXT_XS))
                        .text_color(if stuck || quiet { t.warn } else { t.faint })
                        .child(Clip::new(facts(task))),
                ),
        )
        .tooltip(ui::tip(format!("{} — {}", task.title, facts(task))));
    match pick {
        Pick::Conversation(_) => {
            if let Some(thread) = task.thread.clone() {
                node = node
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.go(Route::Assistant(Some(thread.clone())), cx)
                    }));
            }
        }
        Pick::Agent { run, .. } => {
            let (run, key) = (run.clone(), task.key.clone());
            node = node
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.run_focus = if this
                        .run_focus
                        .as_ref()
                        .is_some_and(|(r, k)| r == &run && k == &key)
                    {
                        None
                    } else {
                        Some((run.clone(), key.clone()))
                    };
                    cx.notify();
                }));
        }
    }
    node.into_any_element()
}

/// What is happening in a run, as one sentence, from the run's state and its tasks.
pub fn status_line(run_state: &str, tasks: &[Value]) -> String {
    flow::status_line(run_state, &tasks_of(tasks))
}

/// How a run is going, in a line, for under a title: "3 of 12 done · 2 working".
pub fn summary(tasks: &[Value]) -> String {
    let (done, total) = flow::progress(&tasks_of(tasks));
    let working = tasks.iter().filter(|t| t["state"] == "running").count();
    let stuck = tasks
        .iter()
        .filter(|t| flow::is_stuck(t["state"].as_str().unwrap_or_default()))
        .count();
    let mut out = format!("{done} of {total} done");
    if working > 0 {
        out.push_str(&format!(" · {working} working"));
    }
    if stuck > 0 {
        out.push_str(&format!(" · {stuck} stopped"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(state: &str, quiet: Option<u64>, attempts: u32) -> Task {
        Task {
            key: "cron".into(),
            role: "implementer".into(),
            title: "Cron".into(),
            state: state.into(),
            thread: None,
            depends_on: Vec::new(),
            quiet_secs: quiet,
            attempts,
        }
    }

    #[test]
    fn the_line_under_a_title_says_the_role_and_where_it_stands_in_words() {
        assert_eq!(facts(&task("running", Some(5), 1)), "implementer · working");
        assert_eq!(
            facts(&task("running", Some(120), 1)),
            "implementer · working, 2 minutes ago"
        );
        assert_eq!(
            facts(&task("running", Some(9 * 60), 1)),
            "implementer · quiet for 9 minutes"
        );
        assert_eq!(facts(&task("pending", None, 1)), "implementer · waiting");
        assert_eq!(
            facts(&task("skipped", None, 1)),
            "implementer · accepted",
            "work accepted by hand reads as accepted, not skipped"
        );
        assert_eq!(
            facts(&task("unverified", None, 2)),
            "implementer · unverified · try 2"
        );
        assert_eq!(
            facts(&task("done", None, 3)),
            "implementer · done",
            "a finished task does not boast its retries"
        );
    }
}
