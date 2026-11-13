//! What happens when a button the assistant drew is pressed.
//!
//! The assistant writes the button; the person decides. Nothing here runs on its own, and nothing the
//! model wrote is trusted to say what it does: a `run` button is looked up in what devd really offers,
//! shown exactly as it is (its id, what it does, its arguments) and, unless it only reads, put to the
//! person to confirm. It then takes the same path as every other button in the window, so tiers,
//! approvals and the activity record all apply.

use devd_app_core::nav::Route;
use devd_app_core::sync::{Mutation, QueryKey};
use gpui::{Context, Window};
use serde_json::Value;

use crate::screens::pages::domain_of;
use crate::shell::Shell;
use crate::state::{devd, Toast};
use crate::ui::genui::Act;
use crate::ui::links::{self, Link};

impl Shell {
    /// Carry out a pressed button, asking first where it matters.
    pub fn ui_action(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        match act {
            Act::Reply(text) => self.reply(&text, window, cx),
            Act::Url(url) => cx.open_url(&url),
            Act::Open(target) => self.open_target(&target, cx),
            Act::Run { capability, args, label } => self.run_drawn(&capability, args, &label, window, cx),
        }
    }

    /// Send a message as the person, leaving whatever they were typing, and what they had attached, as it was.
    fn reply(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.composer.read(cx).text().to_string();
        let files = std::mem::take(&mut self.compose.files);
        self.composer.update(cx, |input, cx| input.set_text(text.to_string(), cx));
        self.send(window, cx);
        self.composer.update(cx, |input, cx| input.set_text(draft, cx));
        self.compose.files = files;
    }

    /// Go to a page of devd, or open a file or folder that exists.
    fn open_target(&mut self, target: &str, cx: &mut Context<Self>) {
        if let Some(route) = Route::parse(target).filter(|_| target.starts_with('/')) {
            return self.go(route, cx);
        }
        let path = std::path::PathBuf::from(target);
        if path.exists() {
            links::open(&Link::File { path, line: None }, cx);
        } else {
            cx.global_mut::<crate::state::Devd>().toasts.push(Toast::new(format!("There is nothing at {target} to open."), true));
        }
    }

    /// Run a capability the assistant offered. It must exist; anything that is not only a read is confirmed.
    fn run_drawn(&mut self, capability: &str, args: Value, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        let known = devd(cx).sync.watch(&QueryKey::bare("api.capabilities"));
        let entry = known.list("").iter().find(|c| c["id"].as_str() == Some(capability)).cloned();
        // While the list is still arriving, nothing is known, so the person is asked.
        let loading = known.is_first_load();
        let (tier, summary) = match &entry {
            Some(c) => (c["tier"].as_str().unwrap_or("write").to_string(), c["summary"].as_str().unwrap_or_default().to_string()),
            None if loading => ("write".to_string(), String::new()),
            None => {
                cx.global_mut::<crate::state::Devd>().toasts.push(Toast::new(format!("devd has nothing called {capability}, so that button does nothing."), true));
                return;
            }
        };
        let mutation = Mutation::new(capability.to_string(), args.clone()).invalidating(domain_of(capability));
        if tier == "read" {
            devd(cx).sync.mutate(mutation);
            return;
        }
        let shown = if args.as_object().is_some_and(|m| m.is_empty()) { String::new() } else { format!("\nWith: {}", clip(&args.to_string(), 500)) };
        let what = if summary.is_empty() { String::new() } else { format!("{summary}.\n") };
        let body = format!("The assistant offered this. It will run `{capability}`.\n{what}Kind of action: {tier}.{shown}");
        let danger = matches!(tier.as_str(), "destructive" | "admin");
        let title = if label.is_empty() { format!("Run {capability}?") } else { format!("{label}?") };
        self.confirm(title, body, "Run it", danger, mutation, window, cx);
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max { text.to_string() } else { format!("{}...", text.chars().take(max).collect::<String>()) }
}
