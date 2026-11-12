//! Intelligence: the models devd thinks with, and what that thinking costs.
//!
//! Three questions are answered here, in the order a person asks them. *What
//! is this costing me?* comes first, because it is the one nobody could answer
//! before. *What can I use?* is each provider with whether it is ready and
//! every model it offers, priced. *What else is there?* is the providers not
//! set up yet, one click from a key.
//!
//! Nothing here is a setting to tune. A provider is ready or it says what
//! stands in the way, and the model list is the provider's own, completed
//! from the open catalogue where the provider is silent.

use devd_app_core::status;
use devd_app_core::sync::Mutation;
use gpui::{div, prelude::*, px, AnyElement, ClickEvent, Context, Div, Entity, Focusable, FontWeight, Subscription, Window};
use serde_json::{json, Value};

use crate::shell::Shell;
use crate::state::{devd, q};
use crate::theme::{mono, size, theme, Tone};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::{self, card, eyebrow, icon, page_head, pill, rule, Button, ButtonKind};

/// What the screen remembers between frames.
pub struct View {
    /// The provider whose models are listed.
    pub open: Option<String>,
    /// The provider being given a key.
    pub adding: Option<String>,
    /// Where the key is pasted. It is drawn masked and never copied out.
    pub key: Entity<TextInput>,
    _watch: Subscription,
}

impl View {
    /// Nothing open, nothing being added.
    pub fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let key = cx.new(|cx| TextInput::new("Paste the API key", cx).secret());
        let watch = cx.subscribe_in(&key, window, |this, _, event: &InputEvent, _, cx| match event {
            InputEvent::Submitted => save_key(this, cx),
            InputEvent::Cancelled => stop_adding(this, cx),
            _ => {}
        });
        Self { open: None, adding: None, key, _watch: watch }
    }
}

fn stop_adding(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.intel.adding = None;
    shell.intel.key.update(cx, |input, cx| input.clear(cx));
    cx.notify();
}

/// Store the pasted key, then ask the provider what that key may use. The
/// second step is the proof: a wrong key is reported by the vendor in its own
/// words a moment later, not discovered in the middle of a conversation.
fn save_key(shell: &mut Shell, cx: &mut Context<Shell>) {
    let Some(id) = shell.intel.adding.clone() else { return };
    let key = shell.intel.key.read(cx).text().trim().to_string();
    if key.is_empty() {
        return;
    }
    let mut args = json!({ "id": id, "api_key": key });
    let is_preset = devd(cx).sync.watch(&q::presets()).list("").iter().any(|p| p["id"] == json!(id));
    if is_preset {
        args["preset"] = json!(id);
    }
    let sync = devd(cx).sync.clone();
    sync.mutate(Mutation::new("provider.save", args).invalidating("models"));
    sync.mutate(Mutation::new("provider.models", json!({ "id": id })));
    shell.intel.open = Some(id);
    stop_adding(shell, cx);
}

/// Dollars as a person reads them: cents when there are any, and never a
/// rounded-down zero for something that cost money.
pub fn money(usd: f64) -> String {
    match usd {
        v if v >= 1000.0 => format!("${:.0}", v),
        v if v >= 0.01 => format!("${:.2}", v),
        v if v > 0.0 => "<$0.01".to_string(),
        _ => "$0.00".to_string(),
    }
}

/// A price per million tokens: `$0.28`, `$15`, and `free` for a model that is.
fn rate(v: &Value) -> Option<String> {
    let v = v.as_f64()?;
    Some(if v == 0.0 {
        "free".to_string()
    } else if v >= 10.0 || v.fract() == 0.0 {
        format!("${v:.0}")
    } else {
        format!("${v:.2}")
    })
}

fn stat(label: &'static str, value: String, detail: String, cx: &Context<Shell>) -> Div {
    let t = theme(cx);
    div()
        .flex_1()
        .min_w_0()
        .px(px(16.0))
        .py(px(14.0))
        .flex()
        .flex_col()
        .gap(px(3.0))
        .child(div().text_size(px(size::TEXT_XS)).text_color(t.faint).child(label))
        .child(div().text_size(px(size::TEXT_XL)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(value))
        .child(div().truncate().text_size(px(size::TEXT_XS)).text_color(t.muted).child(detail))
}

/// What the work has cost, and how much of it the prompt cache paid for.
fn spend(usage: &Value, cx: &Context<Shell>) -> Div {
    let t = theme(cx).clone();
    let unpriced = usage["unpriced_conversations"].as_i64().unwrap_or(0);
    let total = usage["total_cost_usd"].as_f64();
    // An older devd reports no totals; the rows always add up to them.
    let sum = |key: &str| usage["by_model"].as_array().map(|rows| rows.iter().map(|m| m[key].as_u64().unwrap_or(0)).sum::<u64>()).unwrap_or(0);
    let (tokens_in, tokens_out) = (usage["input_tokens"].as_u64().unwrap_or_else(|| sum("input")), usage["output_tokens"].as_u64().unwrap_or_else(|| sum("output")));
    let hit = usage["cache_hit_rate"].as_f64();
    let saved = usage["cache_saved_usd"].as_f64();
    let divider = || div().w(px(1.0)).flex_none().bg(t.line);
    let top = div()
        .flex()
        .child(stat(
            "Estimated spend",
            total.map(money).unwrap_or_else(|| "—".to_string()),
            match (total, unpriced) {
                (None, _) => "No model used so far has a known price".to_string(),
                (_, 0) => "At each provider's list price".to_string(),
                (_, n) => format!("{n} conversation{} could not be priced", if n == 1 { "" } else { "s" }),
            },
            cx,
        ))
        .child(divider())
        .child(stat("Tokens", ui::compact(tokens_in + tokens_out), format!("{} in · {} out", ui::compact(tokens_in), ui::compact(tokens_out)), cx))
        .child(divider())
        .child(stat(
            "Read from cache",
            hit.map(|h| format!("{:.0}%", h * 100.0)).unwrap_or_else(|| "—".to_string()),
            "Of input the provider did not charge in full".to_string(),
            cx,
        ))
        .child(divider())
        .child(stat("Saved by caching", saved.map(money).unwrap_or_else(|| "—".to_string()), "Against the same tokens uncached".to_string(), cx));

    let mut body = card(cx).flex().flex_col().overflow_hidden().child(top);
    let models = usage["by_model"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let tokens = |m: &Value| m["input"].as_u64().unwrap_or(0) + m["output"].as_u64().unwrap_or(0);
    let most = models.iter().map(tokens).max().unwrap_or(0).max(1);
    for model in models.iter().filter(|m| tokens(m) > 0).take(6) {
        let share = tokens(model) as f32 / most as f32;
        body = body.child(rule(cx)).child(
            div()
                .px(px(16.0))
                .h(px(36.0))
                .flex()
                .items_center()
                .gap(px(14.0))
                .child(div().w(px(220.0)).flex_none().truncate().font_family(mono()).text_size(px(size::TEXT_SM)).child(model["name"].as_str().unwrap_or_default().to_string()))
                // The bar is the share of the busiest model, so the eye finds where the tokens went.
                .child(div().flex_1().h(px(4.0)).rounded_full().bg(t.hover).child(div().h_full().w(gpui::relative(share.max(0.01))).rounded_full().bg(t.muted)))
                .child(div().w(px(76.0)).flex_none().flex().justify_end().text_size(px(size::TEXT_SM)).text_color(t.muted).child(ui::compact(tokens(model))))
                .child(
                    div().w(px(72.0)).flex_none().flex().justify_end().font_family(mono()).text_size(px(size::TEXT_SM)).text_color(t.text).child(model["cost_usd"].as_f64().map(money).unwrap_or_else(|| "no price".to_string())),
                ),
        );
    }
    body
}

/// The row a key is pasted into, for a provider that has none.
fn key_form(shell: &Shell, id: &str, label: &str, key_url: Option<String>, cx: &mut Context<Shell>) -> Div {
    let t = theme(cx).clone();
    let mut row = div()
        .px(px(14.0))
        .py(px(12.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .bg(t.raised)
        .child(icon("lock", 14.0, t.faint))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h(px(size::CONTROL))
                .px(px(9.0))
                .flex()
                .items_center()
                .rounded(px(size::RADIUS))
                .border_1()
                .border_color(t.line_strong)
                .bg(t.bg)
                .text_size(px(size::TEXT))
                .child(shell.intel.key.clone()),
        )
        .child(Button::new("intel.key.save", format!("Save and check {label}")).kind(ButtonKind::Primary).on_click(cx.listener(|this, _: &ClickEvent, _, cx| save_key(this, cx))))
        .child(Button::new("intel.key.cancel", "Cancel").kind(ButtonKind::Ghost).on_click(cx.listener(|this, _: &ClickEvent, _, cx| stop_adding(this, cx))));
    if let Some(url) = key_url {
        row = row.child(Button::new("intel.key.get", "Get a key").icon("external").kind(ButtonKind::Ghost).on_click(move |_, _, cx| cx.open_url(&url)));
    }
    let _ = id;
    div().flex().flex_col().child(rule(cx)).child(row).child(
        div().px(px(14.0)).pb(px(10.0)).bg(t.raised).text_size(px(size::TEXT_XS)).text_color(t.faint).child("The key is stored in an owner-only file on this machine. It is never shown again, logged, or sent anywhere but the provider."),
    )
}

/// One provider's models, each with what it can do and what it costs.
fn models(provider: &Value, cx: &mut Context<Shell>) -> Div {
    let t = theme(cx).clone();
    let id = provider["id"].as_str().unwrap_or_default().to_string();
    let chosen = provider["default_model"].as_str().unwrap_or_default().to_string();
    let rows = provider["models"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let mut list = div().flex().flex_col().bg(t.bg);
    if rows.is_empty() {
        return list.child(rule(cx)).child(div().px(px(54.0)).py(px(14.0)).text_size(px(size::TEXT_SM)).text_color(t.muted).child("No models listed yet. Fetch them once a key is in place."));
    }
    let from_catalogue = rows.iter().all(|m| m["source"] == json!("catalogue"));
    if from_catalogue {
        list = list.child(rule(cx)).child(
            div().px(px(54.0)).py(px(8.0)).flex().items_center().gap(px(7.0)).text_size(px(size::TEXT_XS)).text_color(t.faint).child(icon("info", 12.0, t.faint)).child("From the open catalogue at models.dev. Fetching with a key shows exactly what that key may use."),
        );
    }
    const SHOWN: usize = 60;
    for (n, model) in rows.iter().take(SHOWN).enumerate() {
        let model_id = model["model_id"].as_str().unwrap_or_default().to_string();
        let is_chosen = model_id == chosen;
        let mut abilities = div().w(px(150.0)).flex().flex_none().gap(px(4.0));
        for (key, word) in [("tools", "tools"), ("vision", "vision"), ("reasoning", "reasoning")] {
            if model[key] == json!(true) {
                abilities = abilities.child(pill(word, Tone::Neutral, cx));
            }
        }
        if model["tools"] == json!(false) {
            abilities = abilities.child(pill("no tools", Tone::Warn, cx));
        }
        let price = match (rate(&model["input_price"]), rate(&model["output_price"])) {
            (Some(input), Some(output)) if input == "free" && output == "free" => "free".to_string(),
            (Some(input), Some(output)) => format!("{input} / {output}"),
            _ => "no price".to_string(),
        };
        let (provider_id, pick) = (id.clone(), model_id.clone());
        let hover = t.hover;
        let mut name = div().flex_1().min_w_0().flex().items_center().gap(px(8.0)).child(div().truncate().child(model["display"].as_str().unwrap_or(&model_id).to_string()));
        if model["display"].as_str() != Some(model_id.as_str()) {
            name = name.child(div().min_w_0().truncate().font_family(mono()).text_size(px(size::TEXT_XS)).text_color(t.faint).child(model_id.clone()));
        }
        if is_chosen {
            name = name.child(pill("in use", Tone::Accent, cx));
        }
        list = list.child(rule(cx)).child(
            div()
                .id(ui::nid("intel.model", n))
                .pl(px(54.0))
                .pr(px(14.0))
                .h(px(34.0))
                .flex()
                .items_center()
                .gap(px(12.0))
                .text_size(px(size::TEXT_SM))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(name)
                .child(abilities)
                .child(div().w(px(64.0)).flex_none().flex().justify_end().text_color(t.muted).child(model["context_window"].as_u64().map(|c| format!("{} ctx", ui::compact(c))).unwrap_or_default()))
                .child(div().w(px(110.0)).flex_none().flex().justify_end().font_family(mono()).text_color(if price == "no price" { t.faint } else { t.text }).child(price))
                // Choosing a row makes it the model this provider answers with.
                .on_click(move |_, _, cx| {
                    if !is_chosen {
                        devd(cx).sync.mutate(Mutation::new("provider.save", json!({ "id": provider_id, "default_model": pick })).invalidating("models"));
                    }
                }),
        );
    }
    if rows.len() > SHOWN {
        list = list.child(rule(cx)).child(div().px(px(54.0)).py(px(9.0)).text_size(px(size::TEXT_XS)).text_color(t.faint).child(format!("And {} more. Search them from the command menu.", rows.len() - SHOWN)));
    }
    list.child(rule(cx)).child(div().px(px(54.0)).py(px(7.0)).text_size(px(size::TEXT_XS)).text_color(t.faint).child("Prices are US dollars per million tokens, input / output."))
}

/// The screen.
pub fn render(shell: &mut Shell, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let inventory = devd(cx).sync.watch(&q::inventory());
    let usage = devd(cx).sync.watch(&q::usage());
    let presets = devd(cx).sync.watch(&q::presets());
    let providers: Vec<Value> = inventory.list("/providers").to_vec();

    let ready = providers.iter().filter(|p| status::provider(p).level == status::Level::Ok).count();
    let default = providers.iter().find(|p| p["is_default"] == json!(true));
    let headline = match (providers.len(), default) {
        (0, _) if inventory.is_first_load() => "Reading the providers…".to_string(),
        (0, _) => "No provider yet. Add one below and devd's agents can work.".to_string(),
        (n, Some(p)) => format!(
            "{ready} of {n} ready · new conversations use {} from {}",
            p["default_model"].as_str().filter(|m| !m.is_empty()).unwrap_or("the provider's default"),
            p["label"].as_str().unwrap_or_default()
        ),
        (n, None) => format!("{ready} of {n} ready · no default chosen"),
    };
    let fetchable: Vec<String> = providers.iter().filter(|p| p["configured"] == json!(true) && p["enabled"] != json!(false)).filter_map(|p| p["id"].as_str().map(str::to_string)).collect();
    let actions = div()
        .flex()
        .gap(px(8.0))
        .child(Button::new("intel.catalogue", "Update catalogue").icon("refresh").kind(ButtonKind::Ghost).on_click(|_, _, cx| {
            devd(cx).sync.mutate(Mutation::new("models.refresh", json!({})).invalidating("models"));
        }))
        .child(Button::new("intel.fetch", "Fetch models").icon("refresh").disabled(fetchable.is_empty()).on_click(move |_, _, cx| {
            for id in &fetchable {
                devd(cx).sync.mutate(Mutation::new("provider.models", json!({ "id": id })));
            }
        }));
    let mut column = div().flex().flex_col().gap(px(22.0)).child(page_head("Intelligence", headline, actions, cx));

    if let Some(data) = usage.data.as_deref() {
        column = column.child(div().flex().flex_col().gap(px(8.0)).child(eyebrow("SPEND", cx)).child(spend(data, cx)));
    }

    // Providers.
    if inventory.is_first_load() {
        let mut waiting = div().flex().flex_col().gap(px(18.0)).py(px(12.0));
        for n in 0..4 {
            waiting = waiting.child(ui::skeleton(ui::nid("intel.skeleton", n), 280.0 + ((n * 71) % 160) as f32, cx));
        }
        column = column.child(waiting);
    } else if let (Some(failure), true) = (&inventory.error, inventory.data.is_none()) {
        column = column.child(ui::empty("alert", "devd could not answer", failure.message.clone(), cx));
    } else if !providers.is_empty() {
        let mut list = card(cx).flex().flex_col().overflow_hidden();
        for (n, provider) in providers.iter().enumerate() {
            if n > 0 {
                list = list.child(rule(cx));
            }
            let id = provider["id"].as_str().unwrap_or_default().to_string();
            let label = provider["label"].as_str().unwrap_or(&id).to_string();
            let state = status::provider(provider);
            let open = shell.intel.open.as_deref() == Some(id.as_str());
            let adding = shell.intel.adding.as_deref() == Some(id.as_str());
            let brand_name = provider["preset"].as_str().unwrap_or(&id).to_string();
            let count = provider["choice_count"].as_u64().unwrap_or(0);
            let second = match (state.level, provider["fetch_error"].as_str()) {
                // What the vendor said is more use than "not answering".
                (status::Level::Danger, Some(reason)) => reason.to_string(),
                _ => provider["default_model"].as_str().filter(|m| !m.is_empty()).map(str::to_string).unwrap_or_else(|| "No model chosen yet".to_string()),
            };
            let failing = state.level == status::Level::Danger;
            let mut title = div().flex().items_center().gap(px(8.0)).child(div().font_weight(FontWeight::MEDIUM).child(label.clone()));
            if provider["is_default"] == json!(true) {
                title = title.child(pill("default", Tone::Accent, cx));
            }
            let mut buttons = div().flex().flex_none().items_center().gap(px(6.0));
            if state.label == "Needs a key" {
                let target = id.clone();
                buttons = buttons.child(Button::new(ui::nid("intel.addkey", n), "Add key").icon("lock").kind(ButtonKind::Primary).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.intel.adding = Some(target.clone());
                    window.focus(&this.intel.key.focus_handle(cx));
                    cx.notify();
                })));
            } else {
                let (test, fetch) = (id.clone(), id.clone());
                buttons = buttons
                    .child(Button::new(ui::nid("intel.test", n), "Test").kind(ButtonKind::Ghost).on_click(move |_, _, cx| {
                        devd(cx).sync.mutate(Mutation::new("provider.test", json!({ "id": test })).invalidating("models"));
                    }))
                    .child(Button::new(ui::nid("intel.models", n), "Fetch").icon("refresh").kind(ButtonKind::Ghost).on_click(move |_, _, cx| {
                        devd(cx).sync.mutate(Mutation::new("provider.models", json!({ "id": fetch })));
                    }));
                if provider["is_default"] != json!(true) && state.level != status::Level::Danger {
                    let make = id.clone();
                    buttons = buttons.child(Button::new(ui::nid("intel.default", n), "Make default").kind(ButtonKind::Ghost).on_click(move |_, _, cx| {
                        devd(cx).sync.mutate(Mutation::new("provider.default", json!({ "id": make })).invalidating("models"));
                    }));
                }
            }
            let toggle = id.clone();
            let hover = t.hover;
            list = list.child(
                div()
                    .id(ui::nid("intel.provider", n))
                    .px(px(14.0))
                    .h(px(58.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(ui::brand(&brand_name, &label, 28.0, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(1.0))
                            .child(title)
                            .child(if failing {
                                div().truncate().text_size(px(size::TEXT_XS)).text_color(t.danger).child(second)
                            } else {
                                div().truncate().font_family(mono()).text_size(px(size::TEXT_XS)).text_color(t.faint).child(second)
                            }),
                    )
                    .child(div().w(px(84.0)).flex_none().flex().justify_end().text_size(px(size::TEXT_SM)).text_color(t.muted).child(if count == 0 { String::new() } else { format!("{count} model{}", if count == 1 { "" } else { "s" }) }))
                    .child(div().w(px(124.0)).flex_none().flex().items_center().gap(px(7.0)).child(ui::dot(state.level.into(), cx)).child(div().text_size(px(size::TEXT_SM)).text_color(t.tone(state.level.into())).child(state.label.clone())))
                    .child(buttons)
                    .child(icon(if open { "chevron-down" } else { "chevron-right" }, 13.0, t.faint))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.intel.open = if this.intel.open.as_deref() == Some(toggle.as_str()) { None } else { Some(toggle.clone()) };
                        cx.notify();
                    })),
            );
            if adding {
                let key_url = presets.list("").iter().find(|p| p["id"] == json!(brand_name)).and_then(|p| p["key_url"].as_str()).map(str::to_string);
                list = list.child(key_form(shell, &id, &label, key_url, cx));
            }
            if open {
                let remove = id.clone();
                list = list.child(models(provider, cx)).child(rule(cx)).child(
                    div().px(px(14.0)).py(px(9.0)).flex().items_center().justify_end().bg(t.bg).child(
                        Button::new(ui::nid("intel.remove", n), format!("Remove {label}")).kind(ButtonKind::Danger).on_click(move |_, _, cx| {
                            devd(cx).sync.mutate(Mutation::new("provider.remove", json!({ "id": remove })).invalidating("models"));
                        }),
                    ),
                );
            }
        }
        column = column.child(div().flex().flex_col().gap(px(8.0)).child(eyebrow("PROVIDERS", cx)).child(list));
    }

    // What is not set up yet.
    let taken: Vec<&str> = providers.iter().flat_map(|p| [p["id"].as_str(), p["preset"].as_str()]).flatten().collect();
    let offered: Vec<&Value> = presets.list("").iter().filter(|p| p["id"].as_str().is_some_and(|id| id != "custom" && !taken.contains(&id))).collect();
    if !offered.is_empty() {
        let mut grid = div().flex().flex_wrap().gap(px(8.0));
        for (n, preset) in offered.iter().enumerate() {
            let id = preset["id"].as_str().unwrap_or_default().to_string();
            let label = preset["label"].as_str().unwrap_or(&id).to_string();
            let needs_key = preset["requires_key"] == json!(true);
            let selected = shell.intel.adding.as_deref() == Some(id.as_str());
            let kind = match preset["category"].as_str().unwrap_or_default() {
                "local" => "Runs on this machine",
                "aggregator" => "Many vendors, one key",
                _ => "Cloud",
            };
            let target = id.clone();
            let hover = t.hover;
            grid = grid.child(
                div()
                    .id(ui::nid("intel.preset", n))
                    .w(px(222.0))
                    .h(px(52.0))
                    .px(px(11.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(size::RADIUS_L))
                    .border_1()
                    .border_color(if selected { t.accent } else { t.line })
                    .bg(t.surface)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(ui::brand(&id, &label, 28.0, cx))
                    .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().font_weight(FontWeight::MEDIUM).child(label)).child(div().truncate().text_size(px(size::TEXT_XS)).text_color(t.faint).child(kind)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if needs_key {
                            this.intel.adding = Some(target.clone());
                            window.focus(&this.intel.key.focus_handle(cx));
                        } else {
                            // A local server needs no key: add it and look at what it has.
                            let sync = devd(cx).sync.clone();
                            sync.mutate(Mutation::new("provider.save", json!({ "id": target, "preset": target })).invalidating("models"));
                            sync.mutate(Mutation::new("provider.models", json!({ "id": target })));
                            this.intel.open = Some(target.clone());
                        }
                        cx.notify();
                    })),
            );
        }
        let mut section = div().flex().flex_col().gap(px(8.0)).child(eyebrow("ADD A PROVIDER", cx)).child(grid);
        if let Some(preset) = offered.iter().find(|p| p["id"].as_str() == shell.intel.adding.as_deref()) {
            let id = preset["id"].as_str().unwrap_or_default().to_string();
            let label = preset["label"].as_str().unwrap_or(&id).to_string();
            section = section.child(
                card(cx)
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(div().px(px(14.0)).py(px(11.0)).flex().flex_col().gap(px(3.0)).child(div().font_weight(FontWeight::MEDIUM).child(label.clone())).child(div().text_size(px(size::TEXT_SM)).text_color(t.muted).child(preset["notes"].as_str().unwrap_or_default().to_string())))
                    .child(key_form(shell, &id, &label, preset["key_url"].as_str().map(str::to_string), cx)),
            );
        }
        column = column.child(section);
    }
    ui::page("intelligence", 1040.0, column).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_never_rounds_a_cost_down_to_nothing() {
        assert_eq!(money(0.0), "$0.00");
        assert_eq!(money(0.004), "<$0.01");
        assert_eq!(money(12.4), "$12.40");
        assert_eq!(money(1834.2), "$1834");
    }

    #[test]
    fn a_price_reads_the_way_a_vendor_prints_it() {
        assert_eq!(rate(&json!(0.28)).as_deref(), Some("$0.28"));
        assert_eq!(rate(&json!(15.0)).as_deref(), Some("$15"));
        assert_eq!(rate(&json!(0.0)).as_deref(), Some("free"));
        assert_eq!(rate(&Value::Null), None);
    }
}
