//! The assistant's side panels: a browser and a terminal, beside the conversation.
//!
//! The assistant does its work in a shell and a browser, and the person should be able to watch it, step in, and
//! use the same ones. A strip at the edge of the assistant screen opens each panel; both can be open at once, the
//! browser above the terminal. Each panel has its own header: a tab for every page or shell, a button for another,
//! and a way to close the panel. When the assistant starts the browser, its panel opens, so a page it loads is seen as
//! it loads. The shells are real terminals (`terminal.*`) and the browser is the shared Chromium (`browser.*`):
//! the same ones the project's Terminal tab and the assistant's own tools use.

use devd_app_core::surface;
use gpui::{div, prelude::*, px, AnyElement, App, ClickEvent, Context, Div, ElementId, Window};
use serde_json::json;

use super::pages::read_call;
use super::term::{self, Home, TermUi};
use crate::shell::Shell;
use crate::state::{devd, q};
use crate::theme::{size, theme, Theme};
use crate::ui::{self};

/// How wide the panels are.
pub const WIDTH: f32 = 500.0;

/// A panel.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    /// The shells of the folder.
    #[default]
    Terminal,
    /// The shared browser.
    Browser,
}

/// What the panels remember.
#[derive(Default)]
pub struct PaneUi {
    /// Whether the terminal panel is open.
    pub terminal: bool,
    /// Whether the browser panel is open.
    pub browser: bool,
    /// Its shells.
    pub term: TermUi,
    /// Whether the browser was running when last looked at, so a browser that has just started opens its panel.
    browser_was: bool,
}

impl PaneUi {
    /// Open a panel.
    pub fn show(&mut self, tab: Tab) {
        match tab {
            Tab::Terminal => self.terminal = true,
            Tab::Browser => self.browser = true,
        }
    }

    /// Whether any panel is open.
    pub fn any(&self) -> bool {
        self.terminal || self.browser
    }
}

/// A small square button with an icon, for the headers.
pub fn icon_button(id: &'static str, icon: &'static str, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static, t: &Theme) -> AnyElement {
    let hover = t.hover;
    div()
        .id(id)
        .size(px(28.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size::RADIUS))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(ui::icon(icon, 15.0, t.muted))
        .on_click(on_click)
        .into_any_element()
}

/// A tab: a label with a way to close it, and a mark in front when it has one.
#[allow(clippy::too_many_arguments)]
pub fn chip(
    id: ElementId,
    label: String,
    lead: Option<AnyElement>,
    on: bool,
    select: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
    close: impl Fn(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    let hover = t.hover;
    let closer = div()
        .id(ElementId::Name(format!("{id}.x").into()))
        .size(px(18.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .hover(move |s| s.bg(hover))
        .child(ui::icon("x", 11.0, t.faint))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            close(this, window, cx);
            cx.stop_propagation();
        }));
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(7.0))
        .max_w(px(190.0))
        .pl(px(10.0))
        .pr(px(5.0))
        .h(px(30.0))
        .rounded(px(size::RADIUS))
        .cursor_pointer()
        .bg(if on { t.active } else { t.hover.opacity(0.0) })
        .hover(move |s| s.bg(hover))
        .children(lead)
        .child(div().flex_1().min_w_0().truncate().text_size(px(size::TEXT_SM)).text_color(if on { t.text } else { t.muted }).child(label))
        .child(closer)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| select(this, window, cx)))
        .into_any_element()
}

/// A panel's header: its tabs and a button for another on the left, its own controls on the right. With no tab, its name.
pub fn header(name: &'static str, tabs: Vec<AnyElement>, new: AnyElement, right: Vec<AnyElement>, t: &Theme) -> Div {
    let mut left = div().flex().items_center().gap(px(2.0)).flex_1().min_w_0().overflow_hidden();
    if tabs.is_empty() {
        left = left.child(div().px(px(10.0)).text_size(px(size::TEXT_SM)).text_color(t.muted).child(name));
    }
    left = left.children(tabs).child(new);
    div().flex().items_center().justify_between().gap(px(6.0)).px(px(10.0)).h(px(44.0)).flex_none().border_b_1().border_color(t.line).child(left).child(div().flex().items_center().gap(px(2.0)).children(right))
}

/// The folder a shell opens in: the one the conversation is about, else the first project.
fn folder(cx: &Context<Shell>) -> Option<String> {
    if let Some(root) = devd(cx).link_root.as_ref() {
        return Some(root.display().to_string());
    }
    devd(cx).sync.watch(&q::projects()).list("").iter().find_map(|p| p["path"].as_str().map(str::to_string))
}

impl Shell {
    /// Open a panel, or close it when it is open.
    pub fn toggle_pane(&mut self, tab: Tab, cx: &mut Context<Self>) {
        match tab {
            Tab::Terminal => self.pane.terminal = !self.pane.terminal,
            Tab::Browser => self.pane.browser = !self.pane.browser,
        }
        self.present(cx);
        cx.notify();
    }
}

/// The strip at the edge of the screen: one button for the browser and one for the terminal.
pub fn strip(shell: &mut Shell, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    devd(cx).browser_seen.set(true);
    let status = read_call(cx, &surface::browser::status());
    let running = status.data.as_deref().is_some_and(|d| d["running"] == json!(true));
    // The assistant started the browser: show it.
    if running && !shell.pane.browser_was && !shell.pane.browser {
        shell.pane.browser = true;
    }
    shell.pane.browser_was = running;

    let terminals = read_call(cx, &surface::terminal::list());
    let shells = terminals.list("/terminals").iter().any(|s| s["running"] != json!(false));
    let button = |id: &'static str, icon: &'static str, tab: Tab, on: bool, live: bool, cx: &mut Context<Shell>| {
        let hover = t.hover;
        div()
            .id(id)
            .relative()
            .size(px(34.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(size::RADIUS))
            .cursor_pointer()
            .bg(if on { t.active } else { t.hover.opacity(0.0) })
            .hover(move |s| s.bg(hover))
            .child(ui::icon(icon, 16.0, if on { t.text } else { t.muted }))
            .children(live.then(|| div().absolute().top(px(6.0)).right(px(6.0)).size(px(6.0)).rounded_full().bg(t.ok)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_pane(tab, cx)))
    };
    let (browser_on, terminal_on) = (shell.pane.browser, shell.pane.terminal);
    div()
        .w(px(44.0))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.0))
        .pt(px(10.0))
        .border_l_1()
        .border_color(t.line)
        .child(button("pane.browser", "globe", Tab::Browser, browser_on, running, cx))
        .child(button("pane.terminal", "terminal", Tab::Terminal, terminal_on, shells, cx))
        .into_any_element()
}

/// The panels that are open, one above the other.
pub fn pane(shell: &mut Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if !shell.pane.any() {
        return None;
    }
    let t = theme(cx).clone();
    let both = shell.pane.browser && shell.pane.terminal;
    let mut column = div().w(px(WIDTH)).h_full().flex_none().flex().flex_col().border_l_1().border_color(t.line);

    if shell.pane.browser {
        let parts = super::browser::parts(shell, WIDTH, cx);
        let close = icon_button("pane.browser.close", "x", cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_pane(Tab::Browser, cx)), &t);
        let mut panel = div().flex().flex_col().flex_1().min_h_0().child(header("Browser", parts.tabs, parts.new, vec![close], &t));
        panel = panel.child(div().id("pane.browser.body").flex_1().min_h_0().overflow_y_scroll().child(parts.body));
        if both {
            panel = panel.border_b_1().border_color(t.line);
        }
        column = column.child(panel);
    }
    if shell.pane.terminal {
        let panel = match folder(cx) {
            Some(dir) => {
                let parts = term::shells(shell, Home::Pane, &dir, 62, None, cx);
                let close = icon_button("pane.terminal.close", "x", cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_pane(Tab::Terminal, cx)), &t);
                let right: Vec<AnyElement> = parts.copy.into_iter().chain(std::iter::once(close)).collect();
                div().flex().flex_col().flex_1().min_h_0().child(header("Terminal", parts.tabs, parts.new, right, &t)).child(parts.screen)
            }
            None => {
                let close = icon_button("pane.terminal.close", "x", cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_pane(Tab::Terminal, cx)), &t);
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex().items_center().justify_between().px(px(10.0)).h(px(44.0)).border_b_1().border_color(t.line).child(div().px(px(10.0)).text_size(px(size::TEXT_SM)).text_color(t.muted).child("Terminal")).child(close))
                    .child(div().p(px(16.0)).child(ui::empty("terminal", "No project yet", "A terminal opens in a project's folder. Add a project, or open a conversation about one.", cx)))
            }
        };
        column = column.child(panel);
    }
    Some(column.into_any_element())
}
