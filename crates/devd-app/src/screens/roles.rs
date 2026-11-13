//! Adding your own member to the team.
//!
//! A role is a short description of a job: what it is for, the most it may do,
//! what it must leave behind to count as done, who receives its work next, and
//! its instructions. This is the form for one. It sends exactly what an agent's
//! `roles.save` would, and the server checks it the same way: a role that would
//! not load is refused with the reason, which appears as a notice, and nothing
//! is saved.
//!
//! What the person is not asked is what they cannot decide well. What a role
//! must prove follows from what it may do (a role that changes files must show
//! a change and a passing check; one that only reads shows nothing more), and
//! the tier is one of three plain choices.

use devd_app_core::sync::Mutation;
use gpui::{ClickEvent, Context, Div, Entity, Window, div, prelude::*, px};
use serde_json::json;

use crate::shell::Shell;
use crate::state::devd;
use crate::theme::{Tone, size, theme};
use crate::ui::input::TextInput;
use crate::ui::{self, Button, ButtonKind, card, eyebrow, pill, rule};

/// Where in the company a role sits.
const TEAMS: &[(&str, &str)] = &[
    ("engineering", "Engineering"),
    ("design", "Design"),
    ("quality", "Quality and security"),
    ("operations", "Operations"),
    ("knowledge", "Knowledge"),
    ("leadership", "Leadership"),
];

/// What a role may do at most, in plain words.
const TIERS: &[(&str, &str)] = &[
    ("read", "Read only"),
    ("write", "Change files"),
    ("exec", "Change files and run commands"),
];

/// What pressing a choice does to the form.
type Change = Box<dyn Fn(&mut Shell, &mut Context<Shell>)>;

/// The form's state.
pub struct Form {
    /// Whether the form is showing.
    pub open: bool,
    id: Entity<TextInput>,
    title: Entity<TextInput>,
    summary: Entity<TextInput>,
    skills: Entity<TextInput>,
    hands_to: Entity<TextInput>,
    instructions: Entity<TextInput>,
    team: usize,
    tier: usize,
    isolated: bool,
}

impl Form {
    /// An empty form, closed.
    pub fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            open: false,
            id: cx.new(|cx| TextInput::new("support-engineer", cx)),
            title: cx.new(|cx| TextInput::new("Support engineer", cx)),
            summary: cx.new(|cx| TextInput::new("What this member is for, in one line", cx)),
            skills: cx.new(|cx| TextInput::new("fix-bug, plain-speech", cx)),
            hands_to: cx.new(|cx| TextInput::new("qa, reviewer", cx)),
            instructions: cx.new(|cx| {
                TextInput::new(
                    "What it does, what it reads first, how it proves its work",
                    cx,
                )
                .multiline(10)
            }),
            team: 0,
            tier: 2,
            isolated: true,
        }
    }

    /// What it would send, or why it cannot be sent yet.
    fn args(&self, cx: &gpui::App) -> Result<serde_json::Value, &'static str> {
        let text = |e: &Entity<TextInput>| e.read(cx).text().trim().to_string();
        let list = |e: &Entity<TextInput>| -> Vec<String> {
            text(e)
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        let (id, title, summary, instructions) = (
            text(&self.id),
            text(&self.title),
            text(&self.summary),
            text(&self.instructions),
        );
        if id.is_empty() || title.is_empty() || summary.is_empty() {
            return Err("Give it an id, a name and a one-line summary.");
        }
        if instructions.chars().count() < 40 {
            return Err("Write its instructions: what it does and how it proves its work.");
        }
        let tier = TIERS[self.tier].0;
        let writes = tier != "read";
        // What it must leave behind follows from what it may do.
        let proves = match tier {
            "exec" => json!(["change", "verify"]),
            "write" => json!(["change"]),
            _ => json!([]),
        };
        Ok(json!({
            "id": id, "title": title, "team": TEAMS[self.team].0, "summary": summary, "tier": tier,
            "writes": writes, "isolated": self.isolated && writes, "proves": proves,
            "skills": list(&self.skills), "hands_to": list(&self.hands_to), "instructions": instructions,
        }))
    }
}

fn field(label: &'static str, input: &Entity<TextInput>, cx: &Context<Shell>) -> Div {
    let t = theme(cx);
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(7.0))
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_color(t.muted)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .min_h(px(size::CONTROL))
                .px(px(9.0))
                .py(px(3.0))
                .flex()
                .rounded(px(size::RADIUS))
                .border_1()
                .border_color(t.line_strong)
                .bg(t.bg)
                .text_size(px(size::TEXT))
                .child(input.clone()),
        )
}

/// The form, when it is open.
pub fn render(shell: &mut Shell, cx: &mut Context<Shell>) -> Option<Div> {
    if !shell.role_form.open {
        return None;
    }
    let t = theme(cx).clone();
    let form = &shell.role_form;
    let ready = form.args(cx);
    let (team, tier, isolated) = (form.team, form.tier, form.isolated);
    let pick = |id: &'static str, label: String, on_click: Change, cx: &mut Context<Shell>| {
        Button::new(id, label).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            on_click(this, cx);
            cx.notify();
        }))
    };
    let choices = div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .px(px(14.0))
                .py(px(7.0))
                .child(
                    div()
                        .w(px(110.0))
                        .flex_none()
                        .text_color(t.muted)
                        .child("Part of the team"),
                )
                .child(pick(
                    "role.team",
                    TEAMS[team].1.to_string(),
                    Box::new(|this, _| {
                        this.role_form.team = (this.role_form.team + 1) % TEAMS.len()
                    }),
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .px(px(14.0))
                .py(px(7.0))
                .child(
                    div()
                        .w(px(110.0))
                        .flex_none()
                        .text_color(t.muted)
                        .child("It may"),
                )
                .child(pick(
                    "role.tier",
                    TIERS[tier].1.to_string(),
                    Box::new(|this, _| {
                        this.role_form.tier = (this.role_form.tier + 1) % TIERS.len()
                    }),
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .px(px(14.0))
                .py(px(7.0))
                .child(
                    div()
                        .w(px(110.0))
                        .flex_none()
                        .text_color(t.muted)
                        .child("Works in"),
                )
                .child(pick(
                    "role.copy",
                    if isolated {
                        "Its own copy of the project".to_string()
                    } else {
                        "The project itself".to_string()
                    },
                    Box::new(|this, _| this.role_form.isolated = !this.role_form.isolated),
                    cx,
                )),
        );
    let (message, tone) = match &ready {
        Ok(args) => (
            match args["tier"].as_str() {
                Some("exec") => "Its task counts as done when devd's record shows a change and a check that passed.".to_string(),
                Some("write") => "Its task counts as done when devd's record shows a change.".to_string(),
                _ => "It only reads and reports.".to_string(),
            },
            Tone::Neutral,
        ),
        Err(why) => (why.to_string(), Tone::Warn),
    };
    let body = card(cx)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(field("Id", &form.id, cx))
        .child(field("Name", &form.title, cx))
        .child(field("Summary", &form.summary, cx))
        .child(choices)
        .child(field("Skills", &form.skills, cx))
        .child(field("Hands work to", &form.hands_to, cx))
        .child(field("Instructions", &form.instructions, cx))
        .child(rule(cx))
        .child(
            div()
                .px(px(14.0))
                .py(px(10.0))
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.tone(tone))
                        .child(message),
                )
                .child(
                    Button::new("role.cancel", "Cancel")
                        .kind(ButtonKind::Ghost)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.role_form.open = false;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("role.save", "Add to the team")
                        .kind(ButtonKind::Primary)
                        .disabled(ready.is_err())
                        .on_click(
                            cx.listener(move |this, _: &ClickEvent, _: &mut Window, cx| {
                                // devd checks it again: a refusal comes back as a notice with the reason.
                                if let Ok(args) = this.role_form.args(cx) {
                                    devd(cx).sync.mutate(
                                        Mutation::new("roles.save", args).invalidating("team"),
                                    );
                                    this.role_form.open = false;
                                    cx.notify();
                                }
                            }),
                        ),
                ),
        );
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(eyebrow("A MEMBER OF YOUR OWN", cx))
            .child(div().text_size(px(size::TEXT_SM)).text_color(t.muted).child("It joins the team at once. The planner sees it on the roster and can give it tasks; other members can hand work to it."))
            .child(body),
    )
}

/// The mark on a role that is the person's own, with the way to remove it.
pub fn yours(id: &str, n: usize, cx: &mut Context<Shell>) -> Div {
    let target = id.to_string();
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(pill("yours", Tone::Accent, cx))
        .child(
            Button::new(ui::nid("role.remove", n), "Remove")
                .kind(ButtonKind::Ghost)
                .on_click(move |_, _, cx| {
                    devd(cx).sync.mutate(
                        Mutation::new("roles.delete", json!({ "id": target })).invalidating("team"),
                    );
                }),
        )
}
