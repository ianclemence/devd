//! Dialogs: asking before something is undone, asking for what an action needs, and reading one thing in full.
//!
//! Three shapes, one look (the approval dialog's):
//!
//! * **Confirm.** Anything that removes or replaces something says what, says that it cannot be taken
//!   back when that is true, and asks. The button names the act ("Remove runtime"), never "OK".
//! * **Form.** An action that needs a name or an address asks for it in a dialog, with what each field is
//!   for. A required field left empty is said so under the field; nothing is sent until the form is whole.
//!   Enter sends it, Escape puts it away.
//! * **Detail.** One mail message or one skill, read in full where the list only has room for a line.
//!
//! A dialog never decides anything itself: confirming or sending runs an ordinary capability through the
//! same path as a button, so a capability that needs the person's approval still asks for it.

use devd_app_core::sync::{Backend, Mutation, QueryKey};
use gpui::{div, prelude::*, px, AnyElement, App, ClickEvent, Context, Entity, Focusable, FontWeight, MouseButton, SharedString, Subscription, Window};
use serde_json::{json, Map, Value};

use crate::shell::Shell;
use crate::state::devd;
use crate::theme::{size, theme, Tone};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::{self, icon, Button, ButtonKind};

/// What kind of value a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Words.
    Text,
    /// A whole number.
    Number,
    /// Words that are never shown or echoed: a password.
    Secret,
}

/// One thing a form asks for.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// The argument it fills.
    pub key: &'static str,
    /// What it is called.
    pub label: &'static str,
    /// What goes in it, as an example the person can recognise.
    pub hint: &'static str,
    /// What it holds.
    pub kind: Kind,
    /// Whether the action cannot run without it.
    pub required: bool,
}

impl Field {
    /// A required text field.
    pub const fn text(key: &'static str, label: &'static str, hint: &'static str) -> Self {
        Self { key, label, hint, kind: Kind::Text, required: true }
    }

    /// An optional text field.
    pub const fn optional(key: &'static str, label: &'static str, hint: &'static str) -> Self {
        Self { key, label, hint, kind: Kind::Text, required: false }
    }

    /// An optional whole number.
    pub const fn number(key: &'static str, label: &'static str, hint: &'static str) -> Self {
        Self { key, label, hint, kind: Kind::Number, required: false }
    }
}

/// What a form is for: shown, filled in, and turned into a capability call.
#[derive(Debug, Clone)]
pub struct FormSpec {
    /// The dialog's heading.
    pub title: &'static str,
    /// One or two sentences: what this does.
    pub intro: &'static str,
    /// The button, naming the act.
    pub verb: &'static str,
    /// The capability it runs.
    pub capability: &'static str,
    /// The part of devd it changes, so its screens refresh.
    pub domain: &'static str,
    /// Arguments that are always the same.
    pub fixed: Value,
    /// What it asks for.
    pub fields: Vec<Field>,
    /// What to say while it runs, when it can take long enough to notice. `{name}`
    /// is replaced with the first field's text.
    pub busy: Option<&'static str>,
}


/// A form being filled in.
pub struct Form {
    spec: FormSpec,
    inputs: Vec<Entity<TextInput>>,
    error: Option<String>,
    _watch: Vec<Subscription>,
}

/// What a detail dialog shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// One caught mail message.
    Mail,
    /// One skill's instructions.
    Skill,
}

/// The dialog on screen.
pub enum Modal {
    /// Asking before an act.
    Confirm {
        /// The heading.
        title: String,
        /// What will happen.
        body: String,
        /// The button, naming the act.
        verb: String,
        /// Whether it cannot be undone.
        danger: bool,
        /// What runs on yes.
        then: Mutation,
    },
    /// Asking for what an action needs.
    Form(Form),
    /// One thing, read in full.
    Detail {
        /// What it is.
        kind: Detail,
        /// How to fetch it.
        key: QueryKey,
        /// The heading until it arrives.
        title: String,
    },
    /// The window is being closed while work that needs this process is running.
    Closing {
        /// The runs that would stop.
        run_ids: Vec<String>,
    },
}

/// A run id for a scaffold: letters, digits and dashes, unique enough that two
/// windows cannot cancel each other's work.
fn new_run_id() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("ui-{:x}-{:x}", std::process::id(), nanos)
}

/// Turn what was typed into the arguments of the call. An empty optional field is left out; a number that is not a number
/// is refused with a sentence that names the field.
pub fn build_args(spec: &FormSpec, typed: &[String]) -> Result<Value, String> {
    let mut args: Map<String, Value> = spec.fixed.as_object().cloned().unwrap_or_default();
    for (field, text) in spec.fields.iter().zip(typed) {
        let text = text.trim();
        if text.is_empty() {
            if field.required {
                return Err(format!("{} is needed.", field.label));
            }
            continue;
        }
        let value = match field.kind {
            Kind::Number => match text.parse::<i64>() {
                Ok(n) => json!(n),
                Err(_) => return Err(format!("{} has to be a whole number.", field.label)),
            },
            Kind::Text | Kind::Secret => json!(text),
        };
        args.insert(field.key.to_string(), value);
    }
    Ok(Value::Object(args))
}

impl Shell {
    /// Ask before running `then`.
    #[allow(clippy::too_many_arguments)]
    pub fn confirm(&mut self, title: impl Into<String>, body: impl Into<String>, verb: impl Into<String>, danger: bool, then: Mutation, window: &mut Window, cx: &mut Context<Self>) {
        self.modal = Some(Modal::Confirm { title: title.into(), body: body.into(), verb: verb.into(), danger, then });
        // Focus the frame, so Enter and Escape reach the dialog.
        window.focus(&self.focus_handle());
        cx.notify();
    }

    /// Ask for what an action needs.
    pub fn open_form(&mut self, spec: FormSpec, window: &mut Window, cx: &mut Context<Self>) {
        let mut inputs = Vec::new();
        let mut watch = Vec::new();
        for field in &spec.fields {
            let hint = field.hint;
            let secret = field.kind == Kind::Secret;
            let input = cx.new(|cx| {
                let input = TextInput::new(hint, cx);
                if secret {
                    input.secret()
                } else {
                    input
                }
            });
            watch.push(cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => this.submit_form(window, cx),
                InputEvent::Cancelled => this.close_modal(window, cx),
                InputEvent::Changed => {
                    if let Some(Modal::Form(form)) = this.modal.as_mut() {
                        form.error = None;
                    }
                    cx.notify();
                }
                InputEvent::Up | InputEvent::Down => {}
            }));
            inputs.push(input);
        }
        if let Some(first) = inputs.first() {
            window.focus(&first.focus_handle(cx));
        }
        self.modal = Some(Modal::Form(Form { spec, inputs, error: None, _watch: watch }));
        cx.notify();
    }

    /// Show one thing in full.
    pub fn open_detail(&mut self, kind: Detail, key: QueryKey, title: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.modal = Some(Modal::Detail { kind, key, title: title.into() });
        window.focus(&self.focus_handle());
        cx.notify();
    }

    /// Put the dialog away.
    pub fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.take().is_some() {
            window.focus(&self.focus_handle());
            cx.notify();
        }
    }

    /// Ask before the window closes while a scaffold is still running.
    pub fn warn_before_closing(&mut self, run_ids: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.modal = Some(Modal::Closing { run_ids });
        window.focus(&self.focus_handle());
        cx.notify();
    }

    /// Close and stop the scaffolds this window owns. Files already made stay.
    pub fn close_and_cancel(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Modal::Closing { run_ids }) = self.modal.take() {
            let conn = devd(cx).conn.clone();
            for run_id in run_ids {
                let _ = conn.invoke("stacks.cancel", &json!({ "run_id": run_id }));
            }
        }
        devd(cx).allow_close.set(true);
        cx.quit();
    }

    /// Enter: answer yes to a confirmation, or send a form.
    pub fn accept_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.modal.as_ref() {
            Some(Modal::Confirm { .. }) => self.confirm_yes(window, cx),
            Some(Modal::Form(_)) => self.submit_form(window, cx),
            Some(Modal::Closing { .. }) => self.close_and_cancel(window, cx),
            _ => {}
        }
    }

    fn confirm_yes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Modal::Confirm { then, .. }) = self.modal.take() {
            devd(cx).sync.mutate(then);
        }
        window.focus(&self.focus_handle());
        cx.notify();
    }

    /// Send the form, if it is whole.
    pub fn submit_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Modal::Form(form)) = self.modal.as_mut() else { return };
        let typed: Vec<String> = form.inputs.iter().map(|i| i.read(cx).text().to_string()).collect();
        match build_args(&form.spec, &typed) {
            Err(why) => {
                form.error = Some(why);
                cx.notify();
            }
            Ok(mut args) => {
                // A scaffold is followed and cancelled by its run id, and the
                // window names it while it runs. Every other form needs neither.
                let scaffolding = form.spec.capability == "project.init";
                let run_id = scaffolding.then(new_run_id);
                if let (true, Some(id)) = (scaffolding, &run_id) {
                    args["run_id"] = json!(id);
                }
                let note = form.spec.busy.map(|text| {
                    let name = typed.first().map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or("the project");
                    text.replace("{name}", name)
                });
                let mut mutation = Mutation::new(form.spec.capability, args).invalidating(form.spec.domain);
                if scaffolding {
                    // The New project page's run history belongs to the `stacks` domain.
                    mutation = mutation.invalidating("stacks");
                }
                self.modal = None;
                crate::state::start_write(cx, mutation, note, run_id);
                window.focus(&self.focus_handle());
                cx.notify();
            }
        }
    }

    /// The dialog, over everything.
    pub fn modal_overlay(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = theme(cx).clone();
        let modal = self.modal.as_ref()?;
        let panel = match modal {
            Modal::Confirm { title, body, verb, danger, .. } => {
                let (title, body, verb, danger) = (title.clone(), body.clone(), verb.clone(), *danger);
                div()
                    .w(px(480.0))
                    .flex()
                    .flex_col()
                    .gap(px(14.0))
                    .p(px(20.0))
                    .child(div().flex().items_center().gap(px(8.0)).child(icon(if danger { "alert" } else { "info" }, 16.0, if danger { t.danger } else { t.info })).child(div().text_size(px(size::TEXT_LG)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(title)))
                    .child(div().text_size(px(size::TEXT)).line_height(px(21.0)).text_color(t.muted).child(body))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(Button::new("modal.cancel", "Cancel").on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.close_modal(window, cx))))
                            .child(Button::new("modal.confirm", verb).kind(if danger { ButtonKind::Danger } else { ButtonKind::Primary }).on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.confirm_yes(window, cx)))),
                    )
                    .into_any_element()
            }
            Modal::Form(form) => {
                let mut fields = div().flex().flex_col().gap(px(12.0));
                for (n, (field, input)) in form.spec.fields.iter().zip(&form.inputs).enumerate() {
                    fields = fields.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(div().flex().items_center().gap(px(6.0)).child(div().text_size(px(size::TEXT_SM)).font_weight(FontWeight::MEDIUM).text_color(t.text).child(field.label)).child(div().text_size(px(size::TEXT_XS)).text_color(t.faint).child(if field.required { "" } else { "optional" })))
                            .child(
                                div()
                                    .id(ui::nid("modal.field", n))
                                    .h(px(36.0))
                                    .px(px(11.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(size::RADIUS))
                                    .border_1()
                                    .border_color(t.line_strong)
                                    .bg(t.bg)
                                    .text_size(px(size::TEXT))
                                    .line_height(px(22.0))
                                    .child(input.clone()),
                            ),
                    );
                }
                let (title, intro, verb) = (form.spec.title, form.spec.intro, form.spec.verb);
                let error = form.error.clone();
                div()
                    .w(px(500.0))
                    .flex()
                    .flex_col()
                    .gap(px(14.0))
                    .p(px(20.0))
                    .child(div().text_size(px(size::TEXT_LG)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(title))
                    .child(div().text_size(px(size::TEXT)).line_height(px(21.0)).text_color(t.muted).child(intro))
                    .child(fields)
                    .children(error.map(|e| div().flex().items_center().gap(px(7.0)).text_size(px(size::TEXT_SM)).text_color(t.danger).child(icon("alert", 13.0, t.danger)).child(e)))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(Button::new("modal.cancel", "Cancel").on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.close_modal(window, cx))))
                            .child(Button::new("modal.send", verb).kind(ButtonKind::Primary).on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.submit_form(window, cx)))),
                    )
                    .into_any_element()
            }
            Modal::Closing { .. } => div()
                .w(px(480.0))
                .flex()
                .flex_col()
                .gap(px(14.0))
                .p(px(20.0))
                .child(div().flex().items_center().gap(px(8.0)).child(icon("alert", 16.0, t.danger)).child(div().text_size(px(size::TEXT_LG)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child("A project is still being created")))
                .child(div().text_size(px(size::TEXT)).line_height(px(21.0)).text_color(t.muted).child("Closing devd now stops the scaffold. Files already created stay in the folder, but the project is not added to Stacks. You can leave this window open and close it when it has finished."))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(Button::new("modal.keep", "Keep working").on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.close_modal(window, cx))))
                        .child(Button::new("modal.close-cancel", "Close and stop it").kind(ButtonKind::Danger).on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.close_and_cancel(window, cx)))),
                )
                .into_any_element(),
            Modal::Detail { kind, key, title } => {
                let snap = devd(cx).sync.watch(key);
                let body = detail_body(*kind, title, &snap, cx);
                div()
                    .w(px(720.0))
                    .max_h(px(620.0))
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .p(px(20.0))
                    .child(body)
                    .child(div().flex().justify_end().child(Button::new("modal.close", "Close").on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.close_modal(window, cx)))))
                    .into_any_element()
            }
        };
        let panel = div()
            .flex()
            .flex_col()
            .rounded(px(size::RADIUS_L + 2.0))
            .border_1()
            .border_color(t.line_strong)
            .bg(t.raised)
            .shadow_lg()
            .overflow_hidden()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(panel);
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(t.scrim)
                .occlude()
                // A click outside puts the dialog away, as the command menu's does.
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_modal(window, cx)))
                .child(panel)
                .into_any_element(),
        )
    }
}

/// The inside of a detail dialog.
fn detail_body(kind: Detail, title: &str, snap: &devd_app_core::sync::Snapshot, cx: &App) -> AnyElement {
    let t = theme(cx).clone();
    let heading = |text: String| div().text_size(px(size::TEXT_LG)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(text);
    if snap.is_first_load() {
        return div().flex().flex_col().gap(px(12.0)).child(heading(title.to_string())).child(ui::skeleton("modal.detail.a", 420.0, cx)).child(ui::skeleton("modal.detail.b", 300.0, cx)).into_any_element();
    }
    let Some(data) = snap.data.as_deref() else {
        let why = snap.error.as_ref().map(|f| f.message.clone()).unwrap_or_else(|| "devd did not answer.".to_string());
        return div().flex().flex_col().gap(px(10.0)).child(heading(title.to_string())).child(div().text_color(t.danger).child(why)).into_any_element();
    };
    match kind {
        Detail::Mail => {
            let text = |p: &str| data.pointer(p).and_then(Value::as_str).unwrap_or_default().to_string();
            let to = data["to"].as_array().map(|a| a.iter().filter_map(|x| x["Address"].as_str().or(x.as_str())).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            let body = {
                let plain = text("/text");
                if plain.trim().is_empty() { text("/html") } else { plain }
            };
            let row = |label: &'static str, value: String| div().flex().gap(px(10.0)).text_size(px(size::TEXT_SM)).child(div().w(px(48.0)).flex_none().text_color(t.faint).child(label)).child(div().flex_1().min_w_0().text_color(t.text).child(ui::Clip::new(value)));
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .min_h_0()
                .child(heading(text("/subject")))
                .child(div().flex().flex_col().gap(px(4.0)).child(row("From", text("/from"))).child(row("To", to)).child(row("When", ui::ago(&text("/created")))))
                .child(ui::rule(cx))
                .child(div().id("modal.mail.body").flex_1().min_h_0().max_h(px(380.0)).overflow_y_scroll().text_size(px(size::TEXT)).line_height(px(21.0)).text_color(t.text).child(SharedString::from(if body.trim().is_empty() { "This message has no text.".to_string() } else { body })))
                .into_any_element()
        }
        Detail::Skill => {
            let name = data["name"].as_str().unwrap_or(title).to_string();
            let description = data["description"].as_str().unwrap_or_default().to_string();
            let body = data["body"].as_str().unwrap_or_default().to_string();
            let blocks = devd_app_core::markdown::parse(&body);
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .min_h_0()
                .child(div().flex().items_center().gap(px(8.0)).child(heading(name)).child(ui::pill(data["kind"].as_str().unwrap_or("skill").to_string(), Tone::Neutral, cx)))
                .child(div().text_size(px(size::TEXT)).text_color(t.muted).child(description))
                .child(ui::rule(cx))
                .child(div().id("modal.skill.body").flex_1().min_h_0().max_h(px(400.0)).overflow_y_scroll().text_size(px(size::TEXT)).line_height(px(21.0)).child(ui::markdown::document("modal.skill", &blocks, cx)))
                .into_any_element()
        }
    }
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> FormSpec {
        FormSpec {
            title: "Install a runtime",
            intro: "",
            verb: "Install",
            capability: "runtime.install",
            domain: "runtime",
            fixed: json!({ "force": false }),
            fields: vec![Field::text("kind", "Runtime", "php"), Field::text("version", "Version", "8.4"), Field::number("port", "Port", "3306"), Field::optional("note", "Note", "")],
            busy: None,
        }
    }

    fn typed(a: &str, b: &str, c: &str, d: &str) -> Vec<String> {
        vec![a.into(), b.into(), c.into(), d.into()]
    }

    #[test]
    fn a_whole_form_becomes_the_arguments_with_the_fixed_ones_kept() {
        let args = build_args(&spec(), &typed(" php ", "8.4", "3306", "")).unwrap();
        assert_eq!(args, json!({ "force": false, "kind": "php", "version": "8.4", "port": 3306 }), "words are trimmed, the number is a number, an empty optional field is left out");
    }

    #[test]
    fn a_missing_required_field_is_named_and_nothing_is_built() {
        assert_eq!(build_args(&spec(), &typed("php", "  ", "", "")).unwrap_err(), "Version is needed.");
        assert_eq!(build_args(&spec(), &typed("", "8.4", "", "")).unwrap_err(), "Runtime is needed.");
    }

    #[test]
    fn a_number_that_is_not_a_number_is_refused_in_words() {
        assert_eq!(build_args(&spec(), &typed("php", "8.4", "three", "")).unwrap_err(), "Port has to be a whole number.");
        assert!(build_args(&spec(), &typed("php", "8.4", "-1", "")).is_ok(), "a negative whole number is still a whole number");
    }
}
