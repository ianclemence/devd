//! A team run drawn as a chain: each task a card, each dependency a line from the task that finishes first to the one
//! that waits for it.
//!
//! The arithmetic (which column, which row, where each line bends) is `devd_app_core::flow`. This draws it: the lines
//! as smooth curves on a canvas beneath the cards, and the cards as ordinary elements, so a card can be clicked to open
//! the conversation of the agent working on it. A line says what it carries by how it is drawn: solid and green where
//! work has been handed on, bright where it is being handed on now, amber where the work upstream has stopped, and
//! dashed and quiet where nothing has passed yet.

use devd_app_core::flow::{self, Flow, Metrics, Task};
use devd_app_core::nav::Route;
use devd_app_core::status;
use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{
    AnyElement, App, ClickEvent, Context, FontWeight, Hsla, PathBuilder, Pixels, Point,
    ScrollHandle, canvas, div, point, prelude::*, px,
};
use serde_json::Value;

use crate::shell::Shell;
use crate::state::devd;
use crate::theme::{size, theme};
use crate::ui::{self, Clip, role_color, state_mark, tint};

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

/// One line, ready to paint: its points, its colour, its weight and whether it is dashed.
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

/// The graph of a run. `here` is the conversation on screen, whose card is marked; `id` keeps two graphs apart.
pub fn graph(
    id: &'static str,
    tasks: &[Value],
    here: Option<&str>,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    let tasks = tasks_of(tasks);
    // A chain that is still changing is read again as it changes, whoever is driving the run.
    if tasks
        .iter()
        .any(|t| !flow::is_done(&t.state) && t.state != "cancelled" && t.state != "failed")
    {
        devd(cx).run_seen.set(true);
    }
    let m = Metrics::default();
    let layout = flow::layout(&tasks, m);
    if tasks.is_empty() {
        return div().into_any_element();
    }

    // What each line is drawn as.
    let strokes: Vec<Stroke> = layout
        .edges
        .iter()
        .map(|e| {
            let (from, to) = (&tasks[e.from], &tasks[e.to]);
            let (color, width, dashed) = match flow::flow(&from.state, &to.state) {
                Flow::Settled => (tint(t.ok, 0.7), 1.75, false),
                Flow::Live => (t.accent, 2.5, false),
                Flow::Held => (t.warn, 2.0, false),
                Flow::Waiting => (t.line_strong, 1.5, true),
            };
            Stroke {
                points: e.points.clone(),
                color,
                width,
                dashed,
            }
        })
        .collect();
    // The lines that are alive are painted last, so they lie over the quiet ones where they meet.
    let mut strokes = strokes;
    strokes.sort_by_key(|s| s.width as i32);

    let lines = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let at = |p: (f32, f32)| -> Point<Pixels> {
                point(bounds.origin.x + px(p.0), bounds.origin.y + px(p.1))
            };
            for stroke in &strokes {
                let mut path = PathBuilder::stroke(px(stroke.width));
                if stroke.dashed {
                    path = path.dash_array(&[px(4.0), px(4.0)]);
                }
                let first = stroke.points[0];
                path.move_to(at(first));
                for (n, pair) in stroke.points.windows(2).enumerate() {
                    let (a, b) = (pair[0], pair[1]);
                    // Between a card and a lane the line bends; along a lane it runs level.
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
                // The head: a small triangle where the line meets the task that waits.
                let end = *stroke.points.last().unwrap_or(&first);
                let mut head = PathBuilder::fill();
                head.move_to(at(end));
                head.line_to(at((end.0 - 7.0, end.1 - 3.8)));
                head.line_to(at((end.0 - 7.0, end.1 + 3.8)));
                head.close();
                if let Ok(built) = head.build() {
                    window.paint_path(built, stroke.color);
                }
            }
        },
    )
    .absolute()
    .size_full();

    let mut cards: Vec<AnyElement> = Vec::new();
    for (n, node) in layout.nodes.iter().enumerate() {
        cards.push(card(&tasks[n], n, node.center, m, here, cx));
    }

    // A long chain is wider than its place. Bring the part that matters into view when it changes: the task that is
    // working now, or, in a conversation, the agent whose conversation this is. A person who scrolls away is left alone.
    let focus = tasks
        .iter()
        .position(|t| here.is_some() && t.thread.as_deref() == here)
        .or_else(|| frontier(&tasks));
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
    let inner = div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(layout.width))
        .h(px(layout.height))
        .child(lines)
        .children(cards);
    div()
        .id(ui::nid(id, 1))
        .w_full()
        .overflow_x_scroll()
        .track_scroll(&handle)
        .rounded(px(size::RADIUS_L))
        .border_1()
        .border_color(t.line)
        .bg(tint(t.surface, 0.55))
        .child(inner)
        .into_any_element()
}

/// One task.
fn card(
    task: &Task,
    n: usize,
    center: (f32, f32),
    m: Metrics,
    here: Option<&str>,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    let said = status::of(&task.state);
    let tone: crate::theme::Tone = said.level.into();
    let accent = if flow::is_stuck(&task.state) {
        t.tone(tone)
    } else if task.state == "running" {
        t.accent
    } else if flow::is_done(&task.state) {
        tint(t.ok, 0.9)
    } else {
        t.line_strong
    };
    let mine = here.is_some() && task.thread.as_deref() == here;
    let role = role_color(&task.role, cx);
    let hover = t.hover;
    let words: String = task.role.replace(['-', '_'], " ");
    let letter = words
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let mut card = div()
        .id(ui::nid("flow.card", n))
        .absolute()
        .left(px(center.0 - m.node_w / 2.0))
        .top(px(center.1 - m.node_h / 2.0))
        .w(px(m.node_w))
        .h(px(m.node_h))
        .flex()
        .items_center()
        .gap(px(9.0))
        .pl(px(11.0))
        .pr(px(9.0))
        .rounded(px(size::RADIUS_L))
        .border_1()
        .border_color(if mine {
            t.accent
        } else if task.state == "running" {
            tint(t.accent, 0.55)
        } else {
            t.line
        })
        .bg(if task.state == "running" {
            t.raised
        } else {
            t.surface
        })
        .overflow_hidden()
        // A strip down the left edge says where the task stands.
        .child(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .bottom(px(0.0))
                .w(px(3.0))
                .bg(accent),
        )
        .child(
            div()
                .size(px(26.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(tint(role, 0.16))
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(role)
                .child(letter),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .text_size(px(size::TEXT_XS))
                        .text_color(t.faint)
                        .child(Clip::new(words.clone()))
                        // A working task says how long it has been quiet, and that it has been tried before, so a stall shows
                        // on its card and not only to whoever asks.
                        .children(match (task.state.as_str(), task.quiet_secs) {
                            ("running", Some(s)) if s >= flow::QUIET_AFTER => Some(
                                div()
                                    .text_color(t.warn)
                                    .child(format!("quiet {}", flow::span(s))),
                            ),
                            ("running", Some(s)) if s >= 20 => {
                                Some(div().child(format!("{} ago", flow::span(s))))
                            }
                            _ => None,
                        })
                        .children((task.attempts > 1 && !flow::is_done(&task.state)).then(|| {
                            div()
                                .text_color(t.warn)
                                .child(format!("try {}", task.attempts))
                        })),
                )
                .child(
                    div()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.text)
                        .child(Clip::new(task.title.clone())),
                ),
        )
        .child(state_mark(ui::nid("flow.mark", n), &said, cx))
        .tooltip(ui::tip(format!(
            "{} · {} · {}",
            task.role.replace(['-', '_'], " "),
            said.label,
            task.title
        )));
    if let Some(thread) = task.thread.clone() {
        card = card
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.go(Route::Assistant(Some(thread.clone())), cx)
            }));
    }
    card.into_any_element()
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

#[allow(dead_code)]
fn _app(_: &App) {}
