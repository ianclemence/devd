//! Drawing a [`State`] into a frame.
//!
//! Rendering is a pure function of the state: it reads and never writes. That is
//! not just tidiness — it is what lets the tests render into a
//! [`ratatui::backend::TestBackend`] and assert on the resulting buffer, so the
//! panels are verified by looking at them rather than by trusting them.
//!
//! # What is deliberately plain
//!
//! There is no per-row scroll state, no mouse, and no animation. A terminal
//! dashboard's job is to show the truth quickly and to make the safe choice
//! obvious; anything that moves is something the reader has to chase. The cursor
//! row is marked by reverse video, destructive confirmations are marked in red,
//! and everything else is left alone.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::state::{Overlay, PromptKind, State, View};

/// Colours that carry meaning, defined once.
///
/// These are functions rather than constants because a `Style` with a modifier
/// is built by method calls, which cannot run in a `const` initialiser.
/// Whether the reader asked for no colour (`NO_COLOR`, the convention most
/// terminals and CLIs honour). Meaning then has to come from words, symbols and
/// weight alone, which is also the accessible default.
fn monochrome() -> bool {
    std::env::var_os("NO_COLOR")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

fn tone(colour: Color) -> Color {
    if monochrome() {
        Color::Reset
    } else {
        colour
    }
}

/// The cursor row.
fn cursor() -> Style {
    Style::new().add_modifier(Modifier::REVERSED)
}
/// A section heading.
fn heading() -> Style {
    Style::new()
        .fg(tone(Color::Rgb(237, 237, 240)))
        .add_modifier(Modifier::BOLD)
}
/// Machine data: paths, hashes, capability ids.
fn data() -> Style {
    Style::new().fg(tone(Color::Rgb(124, 245, 192)))
}
/// A failure.
fn fail() -> Style {
    Style::new()
        .fg(tone(Color::Rgb(255, 122, 122)))
        .add_modifier(Modifier::BOLD)
}
/// Something that needs the reader's attention but is not a failure.
fn warn() -> Style {
    Style::new().fg(tone(Color::Rgb(242, 201, 76)))
}
/// A success.
fn ok() -> Style {
    Style::new().fg(tone(Color::Rgb(86, 217, 138)))
}
/// Secondary text.
fn quiet() -> Style {
    if monochrome() {
        Style::new().add_modifier(Modifier::DIM)
    } else {
        Style::new().fg(Color::Rgb(138, 138, 148))
    }
}
/// The current tab. Reverse video works on every terminal and does not depend
/// on the reader being able to tell two colours apart.
fn tab_active() -> Style {
    Style::new()
        .fg(tone(Color::Rgb(124, 245, 192)))
        .add_modifier(Modifier::BOLD | Modifier::REVERSED)
}

/// Draw one frame.
pub fn render(frame: &mut Frame, state: &State) {
    let area = frame.area();
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .areas(area);

    render_header(frame, state, header);
    render_body(frame, state, body);
    render_footer(frame, state, footer);
    render_overlay(frame, state, area);
}

fn render_header(frame: &mut Frame, state: &State, area: Rect) {
    let [top, tabs_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    let width = area.width as usize;

    let mut tabs: Vec<Span> = Vec::new();
    for view in View::ALL {
        let style = if view == state.view {
            tab_active()
        } else {
            quiet()
        };
        tabs.push(Span::styled(
            format!(" {} {} ", view.shortcut(), view.title()),
            style,
        ));
    }

    // Identity on the left, the state of the selected repository on the right.
    // The right side is what a reader checks first, so it is allowed to win the
    // space: the home path is shortened rather than pushing it off-screen.
    let summary = match (&state.status, &state.repo) {
        (Some(status), Some(repo)) => format!(
            "{}  {}",
            truncate(repo, 28),
            status.summary()
        ),
        (Some(status), None) => status.summary(),
        _ => "no repository selected".to_string(),
    };
    let summary_width = summary.chars().count().min(width / 2);
    let left_width = width.saturating_sub(summary_width + 1);
    let [left, right] = Layout::horizontal([
        Constraint::Length(left_width as u16),
        Constraint::Min(1),
    ])
    .areas(top);
    let prefix = format!(" devd {}  ", state.version);
    let suffix = format!("  git {}", state.git_version);
    let room = left_width.saturating_sub(prefix.chars().count() + suffix.chars().count());
    let identity = Line::from(vec![
        Span::styled(" devd ", heading()),
        Span::styled(state.version.clone(), data()),
        Span::raw("  "),
        Span::styled(truncate(&state.home, room.max(8)), quiet()),
        Span::styled(suffix, quiet()),
    ]);
    frame.render_widget(Paragraph::new(identity), left);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{summary} "),
            quiet(),
        )))
        .alignment(ratatui::layout::Alignment::Right),
        right,
    );
    frame.render_widget(Paragraph::new(Line::from(tabs)), tabs_area);
}

fn render_body(frame: &mut Frame, state: &State, area: Rect) {
    let (title, lines) = match state.view {
        View::Dashboard => dashboard(state),
        View::Projects => list(
            state,
            "Projects",
            state
                .projects
                .iter()
                .map(|p| {
                    Line::from(vec![
                        Span::raw(format!("{:<20}", truncate(&p.name, 20))),
                        Span::styled(truncate(&p.path, 60), data()),
                        Span::styled(
                            format!(
                                "  {}{}",
                                p.default_branch.as_deref().unwrap_or(""),
                                p.remote_url
                                    .as_deref()
                                    .map(|r| format!("  {r}"))
                                    .unwrap_or_default()
                            ),
                            quiet(),
                        ),
                    ])
                })
                .collect(),
        ),
        View::Changes => changes(state),
        View::History => list(
            state,
            "History",
            state
                .commits
                .iter()
                .map(|c| {
                    Line::from(vec![
                        Span::styled(format!("{} ", c.short), data()),
                        Span::raw(truncate(&c.subject, 62)),
                        Span::styled(format!("  {}", c.author), quiet()),
                    ])
                })
                .collect(),
        ),
        View::Branches => list(
            state,
            "Branches",
            state
                .branches
                .iter()
                .map(|b| {
                    Line::from(vec![
                        Span::styled(
                            if b.current { "* " } else { "  " }.to_string(),
                            if b.current { ok() } else { quiet() },
                        ),
                        Span::raw(format!("{:<28}", truncate(&b.name, 28))),
                        Span::styled(
                            b.upstream.clone().unwrap_or_else(|| "-".to_string()),
                            quiet(),
                        ),
                        Span::styled(
                            b.subject
                                .as_deref()
                                .map(|s| format!("  {}", truncate(s, 40)))
                                .unwrap_or_default(),
                            quiet(),
                        ),
                    ])
                })
                .collect(),
        ),
        View::Create => list(
            state,
            "Create a project - Enter to start",
            state
                .stacks
                .iter()
                .map(|s| {
                    Line::from(vec![
                        Span::raw(format!("{:<20}", truncate(&s.name, 20))),
                        Span::styled(format!("{:<12}", truncate(&s.language, 12)), data()),
                        Span::styled(format!("{:<10}", truncate(&s.category, 10)), quiet()),
                        Span::raw(truncate(&s.summary, 70)),
                    ])
                })
                .collect(),
        ),
        View::Toolchain => list(
            state,
            "Toolchain",
            state
                .tools
                .iter()
                .map(|t| {
                    let (mark, style) = if !t.installed {
                        ("missing  ", warn())
                    } else if t.outdated {
                        ("outdated ", warn())
                    } else {
                        ("installed", ok())
                    };
                    let service = match (t.service_healthy, t.port) {
                        (Some(true), Some(p)) => format!("running :{p}"),
                        (Some(false), Some(p)) if t.installed => format!("stopped :{p}"),
                        _ => String::new(),
                    };
                    Line::from(vec![
                        Span::raw(format!("{:<24}", truncate(&t.name, 24))),
                        Span::styled(format!("{mark:<10}"), style),
                        Span::styled(format!("{:<12}", truncate(t.version.as_deref().unwrap_or("-"), 12)), data()),
                        Span::styled(format!("{:<16}", truncate(&service, 16)), if service.starts_with("running") { ok() } else { quiet() }),
                        Span::styled(truncate(&t.category, 16), quiet()),
                    ])
                })
                .collect(),
        ),
        View::Capabilities => list(
            state,
            &format!("Capabilities matching `{}`", state.capability_query),
            state
                .capabilities
                .iter()
                .map(|c| {
                    let style = if c.requires_approval { warn() } else { quiet() };
                    Line::from(vec![
                        Span::raw(format!("{:<12}", c.tier)),
                        Span::styled(format!("{:<26}", truncate(&c.id, 26)), data()),
                        Span::raw(truncate(&c.summary, 60)),
                        Span::styled(
                            if c.requires_approval {
                                "  approval"
                            } else {
                                ""
                            }
                            .to_string(),
                            style,
                        ),
                    ])
                })
                .collect(),
        ),
        View::Team => list(
            state,
            "Team runs",
            state
                .runs
                .iter()
                .map(|r| {
                    let st = r["state"].as_str().unwrap_or("");
                    let style = match st {
                        "done" => ok(),
                        "failed" => fail(),
                        "awaiting_plan" | "paused" => warn(),
                        _ => quiet(),
                    };
                    Line::from(vec![
                        Span::styled(format!("{:<14}", st.replace('_', " ")), style),
                        Span::styled(format!("{:>3}/{:<3} ", r["tasks_done"], r["tasks_total"]), data()),
                        Span::raw(truncate(r["title"].as_str().unwrap_or(""), 70)),
                    ])
                })
                .collect(),
        ),
        View::Inbox => list(
            state,
            "Waiting for you",
            state
                .inbox
                .iter()
                .map(|r| {
                    Line::from(vec![
                        Span::styled(format!("{:<10}", r["kind"].as_str().unwrap_or("")), warn()),
                        Span::styled(format!("{:<28}", truncate(r["title"].as_str().unwrap_or(""), 28)), data()),
                        Span::raw(truncate(r["detail"].as_str().unwrap_or(""), 70)),
                    ])
                })
                .collect(),
        ),
        View::Activity => list(
            state,
            "Activity",
            state
                .audit
                .iter()
                .map(|a| {
                    // A plan is not an application, and a refusal is neither.
                    // Labelling all three is the difference between an audit
                    // trail and a list of rows.
                    let (label, style) = if !a.ok {
                        ("refused", fail())
                    } else if a.dry_run {
                        ("plan", quiet())
                    } else {
                        ("applied", ok())
                    };
                    Line::from(vec![
                        Span::styled(truncate(&a.at, 19), quiet()),
                        Span::raw("  "),
                        Span::raw(format!("{:<16}", truncate(&a.actor, 16))),
                        Span::styled(format!("{:<24}", truncate(&a.capability, 24)), data()),
                        Span::styled(format!("{label:<8}"), style),
                        Span::raw(truncate(&a.message, 60)),
                    ])
                })
                .collect(),
        ),
        View::Doctor => list(
            state,
            "Doctor",
            state
                .checks
                .iter()
                .map(|c| {
                    let style = match c.level.as_str() {
                        "fail" => fail(),
                        "info" => warn(),
                        _ => ok(),
                    };
                    Line::from(vec![
                        Span::styled(format!("{:<5}", c.level), style),
                        Span::raw(format!("{:<16}", truncate(&c.name, 16))),
                        Span::styled(c.detail.clone(), quiet()),
                    ])
                })
                .collect(),
        ),
        View::Help => ("Help".to_string(), help_lines()),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(format!(" {title} "));

    if state.view == View::Dashboard || state.view == View::Help {
        frame.render_widget(Paragraph::new(lines).block(block), area);
        return;
    }

    // Lists: slice the window out of the rows so the cursor is always visible.
    let height = area.height.saturating_sub(2) as usize;
    let first = window(lines.len(), state.cursor, state.scroll, height);
    let visible: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(index, line)| {
            if index == state.cursor {
                line.style(cursor())
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(visible).block(block), area);
}

/// A list view with a count in its title and an honest empty state.
fn list(state: &State, title: &str, rows: Vec<Line<'static>>) -> (String, Vec<Line<'static>>) {
    let title = format!("{title} ({})", rows.len());
    if rows.is_empty() {
        return (
            title,
            vec![
                empty_line(state),
                Line::from(""),
                Line::from(Span::styled(
                    format!("  {}", empty_hint(state)),
                    quiet(),
                )),
            ],
        );
    }
    (title, rows)
}

/// A second line for an empty panel: what to press, not just what is missing.
fn empty_hint(state: &State) -> &'static str {
    match state.view {
        View::Projects => "press t to track a repository by path",
        View::Changes if state.repo.is_none() => "press 2 to choose a project, or t to add one",
        View::Changes => "nothing to stage; press c to commit the last state",
        View::History if state.repo.is_none() => "press 2 to choose a project first",
        View::History => "commits appear here after the first one",
        View::Branches if state.repo.is_none() => "press 2 to choose a project first",
        View::Branches => "press n to create a branch",
        View::Capabilities => "press / and type part of a capability id",
        View::Activity => "operations are recorded as they run",
        View::Team => "devd team start \"what you want built\" begins one",
        View::Inbox => "approvals and questions from agents arrive here",
        View::Doctor => "press r to run the checks again",
        _ => "press ? for the keys that apply here",
    }
}

fn empty_line(state: &State) -> Line<'static> {
    let text = match state.view {
        View::Projects => "No repositories yet - press t to add a path,".to_string(),
        View::Changes if state.repo.is_none() => {
            "No repository selected - press 2 to choose one, or t to add a path".to_string()
        }
        View::Changes => "Nothing changed, or nothing to show yet".to_string(),
        View::History if state.repo.is_none() => "No repository selected".to_string(),
        View::History => "No commits yet".to_string(),
        View::Branches if state.repo.is_none() => "No repository selected".to_string(),
        View::Branches => "No branches".to_string(),
        View::Capabilities => "No capability matches - press / to search again".to_string(),
        View::Activity => "Nothing has happened yet".to_string(),
        View::Team => "No team runs yet. Start one with `devd team start \"what you want built\"` or in the app.".to_string(),
        View::Inbox => "Nothing is waiting for you.".to_string(),
        View::Doctor => "No checks ran".to_string(),
        _ => "Nothing to show".to_string(),
    };
    Line::from(Span::styled(text, quiet()))
}

fn changes(state: &State) -> (String, Vec<Line<'static>>) {
    let Some(status) = &state.status else {
        return list(state, "Changes", Vec::new());
    };

    let items = status.items();
    let title = format!(
        "Changes - {} in {}",
        status.summary(),
        state.repo.as_deref().unwrap_or("(no repository)")
    );

    if items.is_empty() {
        let mut lines = vec![Line::from(Span::styled("The working tree is clean", ok()))];
        if status.ahead > 0 || status.behind > 0 {
            lines.push(Line::from(Span::styled(
                format!(
                    "{} ahead, {} behind {}",
                    status.ahead,
                    status.behind,
                    status.upstream.clone().unwrap_or_default()
                ),
                quiet(),
            )));
        }
        return (title, lines);
    }

    let rows = items
        .iter()
        .map(|item| {
            let style = match item.bucket {
                crate::state::Bucket::Conflicted => fail(),
                crate::state::Bucket::Staged => ok(),
                crate::state::Bucket::Unstaged => warn(),
                crate::state::Bucket::Untracked => quiet(),
            };
            Line::from(vec![
                Span::styled(format!("{} ", item.code), style),
                Span::styled(format!("{:<11}", item.bucket.label()), quiet()),
                Span::raw(item.path.clone()),
            ])
        })
        .collect();
    (title, rows)
}

fn dashboard(state: &State) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();

    let failing = state.failing_checks();
    // One sentence first: what a reader wants to know before anything else.
    let (summary, summary_style) = if state.checks.is_empty() {
        ("environment not checked yet".to_string(), quiet())
    } else if failing > 0 {
        (format!("{failing} check(s) need attention"), fail())
    } else {
        ("environment is healthy".to_string(), ok())
    };
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(summary, summary_style),
    ]));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Environment", heading())));
    if state.checks.is_empty() {
        lines.push(Line::from(Span::styled("  no checks have run", quiet())));
    } else {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{} checks", state.checks.len()),
                if failing > 0 { fail() } else { ok() },
            ),
            Span::styled(
                if failing > 0 {
                    format!(", {failing} failing")
                } else {
                    String::new()
                },
                fail(),
            ),
        ]));
        for check in state.checks.iter().filter(|c| c.level != "ok") {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:<5}", check.level), warn()),
                Span::styled(format!("{:<16}", check.name), quiet()),
                Span::styled(truncate(&check.detail, 70), quiet()),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Repository", heading())));
    match &state.status {
        Some(status) => {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(state.repo.clone().unwrap_or_default(), data()),
            ]));
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::raw(status.summary()),
            ]));
            if status.in_progress.is_some() || !status.conflicted.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!(
                        "  {} - palette: press 3 to see the conflicts",
                        status
                            .in_progress
                            .clone()
                            .unwrap_or_else(|| "unmerged paths".to_string())
                    ),
                    fail(),
                )));
            }
            if status.ahead > 0 || status.behind > 0 {
                lines.push(Line::from(Span::styled(
                    format!("  {} ahead, {} behind", status.ahead, status.behind),
                    quiet(),
                )));
            }
        }
        None => lines.push(Line::from(Span::styled(
            "  no repository selected - press 2 to choose one",
            quiet(),
        ))),
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Projects", heading())));
    lines.push(Line::from(Span::styled(
        format!("  {} tracked", state.projects.len()),
        quiet(),
    )));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Registry", heading())));
    lines.push(Line::from(Span::styled(
        format!(
            "  {} capabilities, as of the last search - press 6 for the list",
            state.capabilities.len()
        ),
        quiet(),
    )));

    ("Dashboard".to_string(), lines)
}

fn help_lines() -> Vec<Line<'static>> {
    let rows: Vec<(&str, &str)> = vec![
        ("1..9, 0, -", "jump to a view (0 = Create, - = Toolchain)"),
        ("Create view", "Enter starts a guided flow for the stack; every question comes from its definition, and the exact command is shown before anything runs"),
        ("Toolchain view", "i install, x remove, s start, t stop, l logs; installs ask first"),
        ("Projects view", "d detect the stack and tasks, m import for agents"),
        ("Tab / Shift+Tab", "next / previous view"),
        ("j / k, arrows", "move the cursor"),
        (
            "Enter",
            "open: select a project, toggle a file in the index, show a commit, switch a branch",
        ),
        ("s / u", "stage / unstage the selected file"),
        ("d", "discard the selected file (asks first)"),
        ("a", "stage everything"),
        ("c", "commit (asks for a message)"),
        ("n", "new branch"),
        ("t", "track a repository by path"),
        ("/", "search capabilities"),
        ("x", "run the selected capability (asks for arguments)"),
        ("y / n", "approve / decline the operation being asked about"),
        ("r", "reload the current view"),
        ("? / q", "this help / quit"),
    ];
    let mut lines = vec![Line::from(Span::styled(
        "Keys apply to the view you are in. Nothing that destroys work happens without a plan and a yes.",
        quiet(),
    ))];
    lines.push(Line::from(""));
    for (key, meaning) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<18}", key), data()),
            Span::raw(meaning.to_string()),
        ]));
    }
    lines
}

fn render_footer(frame: &mut Frame, state: &State, area: Rect) {
    let width = area.width as usize;
    let (glyph, style) = if state.message_is_error {
        ("!", fail())
    } else {
        ("·", quiet())
    };
    // One line for what just happened, one for what can be pressed next. Both
    // are cut to the terminal width rather than wrapping into the hint line.
    let message = Line::from(vec![
        Span::styled(format!(" {glyph} "), style),
        Span::styled(truncate(&state.message, width.saturating_sub(3)), style),
    ]);
    let hints = Line::from(Span::styled(
        format!(" {}", truncate(hints_for(state.view), width.saturating_sub(2))),
        quiet(),
    ));
    frame.render_widget(Paragraph::new(vec![message, hints]), area);
}

fn hints_for(view: View) -> &'static str {
    match view {
        View::Projects => "t add   Enter open   d detect   m import   r refresh   ? help   q quit",
        View::Changes => {
            "Enter stage/unstage   s stage   u unstage   d discard   a stage all   c commit   r refresh   ? help   q quit"
        }
        View::History => "Enter show   j/k move   r refresh   ? help   q quit",
        View::Branches => "Enter switch   n new branch   r refresh   ? help   q quit",
        View::Capabilities => "/ search   Enter describe   x run   r refresh   ? help   q quit",
        View::Create => "Enter create from this stack   r refresh   ? help   q quit",
        View::Toolchain => "Enter details   i install   x remove   s start   t stop   l logs   r refresh   q quit",
        View::Team => "Enter details   a approve plan   p pause/resume   c cancel   r refresh   ? help   q quit",
        View::Inbox => "Enter details   a approve   d decline   r refresh   ? help   q quit",
        _ => "j/k move   Tab next view   r refresh   ? help   q quit",
    }
}

fn render_overlay(frame: &mut Frame, state: &State, area: Rect) {
    match &state.overlay {
        Overlay::None => {}
        Overlay::Prompt { kind, input } => {
            let rect = centered(area, area.width.saturating_sub(8).min(90), 5);
            frame.render_widget(Clear, rect);
            let body = Line::from(vec![
                Span::raw(" "),
                Span::raw(input.clone()),
                Span::styled("_", quiet()),
            ]);
            let hint = match kind {
                PromptKind::CapabilityArgs => {
                    "JSON object, e.g. {\"repo\":\"D:/app\"}   Enter run   Esc cancel"
                }
                _ => "Enter accept   Esc cancel",
            };
            let block = Block::default()
                .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
                .title(format!(" {} ", kind.title()));
            frame.render_widget(
                Paragraph::new(vec![
                    body,
                    Line::from(""),
                    Line::from(Span::styled(format!(" {hint}"), quiet())),
                ])
                .block(block),
                rect,
            );
        }
        Overlay::Form { title, lines, input } => {
            let height = (lines.len() as u16 + 6).min(area.height.saturating_sub(2));
            let rect = centered(area, area.width.saturating_sub(8).min(100), height);
            frame.render_widget(Clear, rect);
            let mut body: Vec<Line> = lines.iter().map(|l| Line::from(format!(" {l}"))).collect();
            body.push(Line::from(""));
            body.push(Line::from(vec![Span::styled(" > ", data()), Span::raw(input.clone()), Span::styled("_", quiet())]));
            body.push(Line::from(Span::styled(" Enter accept (blank keeps the default)   Esc cancel", quiet())));
            frame.render_widget(
                Paragraph::new(body).wrap(Wrap { trim: false }).block(Block::default().borders(Borders::ALL)
        .border_type(BorderType::Rounded).title(format!(" {title} "))),
                rect,
            );
        }
        Overlay::Detail {
            title,
            lines,
            scroll,
        } => {
            let rect = centered(
                area,
                area.width.saturating_sub(6),
                area.height.saturating_sub(4),
            );
            frame.render_widget(Clear, rect);
            let rendered: Vec<Line> = lines.iter().map(|l| Line::from(l.clone())).collect();
            let block = Block::default()
                .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
                .title(format!(" {title} - j/k scroll, Esc to close "));
            frame.render_widget(
                Paragraph::new(rendered)
                    .block(block)
                    .wrap(Wrap { trim: false })
                    .scroll((*scroll, 0)),
                rect,
            );
        }
        Overlay::Approval(pending) => {
            let rect = centered(area, area.width.saturating_sub(12).min(96), 14);
            frame.render_widget(Clear, rect);

            let destructive = pending
                .plan
                .as_ref()
                .map(|p| p.destructive)
                .unwrap_or(pending.plan.is_none());
            let mut lines: Vec<Line> = Vec::new();
            lines.push(Line::from(vec![
                Span::styled(" ", quiet()),
                Span::styled(pending.id.clone(), data()),
                Span::styled(" needs your decision before it runs", quiet()),
            ]));
            lines.push(Line::from(""));

            match &pending.plan {
                Some(plan) => {
                    lines.push(Line::from(Span::styled(
                        format!(" {}", plan.summary),
                        heading(),
                    )));
                    for step in &plan.steps {
                        lines.push(Line::from(format!("   - {step}")));
                    }
                    for effect in &plan.effects {
                        lines.push(Line::from(Span::styled(format!("   ! {effect}"), warn())));
                    }
                }
                None => lines.push(Line::from(Span::styled(
                    " This operation cannot describe itself in advance. It will still be recorded.",
                    warn(),
                ))),
            }

            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(" arguments ", quiet()),
                Span::raw(sanitise(&pending.request.args.to_string())),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(" y ", ok()),
                Span::raw("approve and run      "),
                Span::styled(" n ", fail()),
                Span::raw("decline, and record the refusal"),
            ]));

            let block = Block::default()
                .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
                .title(if destructive {
                    " Confirm a destructive operation "
                } else {
                    " Confirm "
                });
            let block = if destructive {
                block.border_style(fail())
            } else {
                block
            };
            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .block(block),
                rect,
            );
        }
    }
}

/// A rectangle centred in `area`, never larger than it.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// The first row to draw, keeping the cursor inside a window of `height` rows.
fn window(count: usize, cursor: usize, desired: usize, height: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let height = height.max(1);
    // Never scrolled past the cursor, and never leaving it below the window.
    let mut first = desired.min(cursor);
    if cursor >= first + height {
        first = cursor + 1 - height;
    }
    first.min(count.saturating_sub(1))
}

/// Make text safe to draw.
///
/// A path or a branch name can contain a control character, and a control
/// character written to a terminal is not text, it is a command. Git refuses
/// most of these, but "most" is not a guarantee, and this is one line to remove
/// the question.
fn sanitise(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// Shorten to `width` characters, marking that it was shortened.
fn truncate(text: &str, width: usize) -> String {
    let cleaned = sanitise(text);
    if cleaned.chars().count() <= width {
        return cleaned;
    }
    let kept: String = cleaned.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::state::{
        AuditRow, BranchRow, CapRow, CheckRow, CommitRow, Overlay, Pending, RepoRow, StatusData,
    };

    fn draw(state: &State) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|frame| render(frame, state)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let area = buffer.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    out.push_str(cell.symbol());
                }
            }
            out.push('\n');
        }
        out
    }

    fn state_with_status() -> State {
        let mut state = State::default();
        state.home = "D:\\devd-test-home".to_string();
        state.git_version = "2.46.0.windows.1".to_string();
        state.repo = Some("D:/tmp/demo".to_string());
        state.status = Some(StatusData::from_json(&serde_json::json!({
            "branch": "main",
            "clean": false,
            "staged": [{ "status": "M", "path": "src/main.rs" }],
            "unstaged": [{ "status": "M", "path": "README.md" }],
            "untracked": ["notes.txt"],
            "conflicted": [],
            "ahead": 1,
            "behind": 0,
            "unborn": false,
            "in_progress": null
        })));
        state
    }

    #[test]
    fn the_header_shows_the_version_the_home_and_git() {
        let mut state = state_with_status();
        state.view = View::Dashboard;
        let screen = draw(&state);
        assert!(screen.contains("devd"));
        assert!(screen.contains("0.3.0") || screen.contains(env!("CARGO_PKG_VERSION")));
        assert!(screen.contains("devd-test-home"));
        assert!(screen.contains("2.46"));
        assert!(screen.contains("Dashboard"), "the tab strip is missing");
    }

    #[test]
    fn the_empty_states_are_sentences_not_blanks() {
        let mut state = State::default();
        state.view = View::Changes;
        let screen = draw(&state);
        assert!(
            screen.contains("No repository selected"),
            "an empty Changes view said nothing:\n{screen}"
        );

        state.view = View::Branches;
        let screen = draw(&state);
        assert!(screen.contains("No repository selected"), "{screen}");
    }

    #[test]
    fn the_changes_panel_shows_the_branch_and_each_file() {
        let mut state = state_with_status();
        state.view = View::Changes;
        let screen = draw(&state);
        assert!(
            screen.contains("main: 1 staged, 1 unstaged, 1 untracked"),
            "{screen}"
        );
        assert!(screen.contains("src/main.rs"), "{screen}");
        assert!(screen.contains("README.md"), "{screen}");
        assert!(screen.contains("notes.txt"), "{screen}");
        assert!(screen.contains("staged"), "{screen}");
        assert!(screen.contains("untracked"), "{screen}");
        // The footer tells the reader what the keys do.
        assert!(screen.contains("d discard"), "{screen}");
    }

    #[test]
    fn a_clean_tree_says_so_rather_than_showing_nothing() {
        let mut state = State::default();
        state.repo = Some("D:/tmp/demo".to_string());
        state.status = Some(StatusData::default());
        state.view = View::Changes;
        let screen = draw(&state);
        assert!(screen.contains("clean"), "{screen}");
    }

    #[test]
    fn history_shows_the_short_hash_and_the_subject() {
        let mut state = State::default();
        state.view = View::History;
        state.commits = vec![CommitRow {
            short: "abc1234".to_string(),
            subject: "Add the thing".to_string(),
            author: "Ian".to_string(),
            ..Default::default()
        }];
        let screen = draw(&state);
        assert!(screen.contains("abc1234"), "{screen}");
        assert!(screen.contains("Add the thing"), "{screen}");
    }

    #[test]
    fn branches_mark_the_current_one() {
        let mut state = State::default();
        state.view = View::Branches;
        state.branches = vec![
            BranchRow {
                name: "main".to_string(),
                current: true,
                ..Default::default()
            },
            BranchRow {
                name: "feature".to_string(),
                current: false,
                ..Default::default()
            },
        ];
        let screen = draw(&state);
        assert!(screen.contains("* main"), "{screen}");
        assert!(screen.contains("  feature"), "{screen}");
    }

    #[test]
    fn the_activity_panel_distinguishes_a_plan_from_an_application_and_a_refusal() {
        let mut state = State::default();
        state.view = View::Activity;
        state.audit = vec![
            AuditRow {
                at: "2026-10-19T00:00:00Z".to_string(),
                actor: "human:tui".to_string(),
                capability: "git.discard".to_string(),
                tier: "destructive".to_string(),
                ok: false,
                dry_run: false,
                message: "declined by the operator".to_string(),
            },
            AuditRow {
                at: "2026-10-19T00:00:01Z".to_string(),
                actor: "human:tui".to_string(),
                capability: "git.reset".to_string(),
                tier: "destructive".to_string(),
                ok: true,
                dry_run: true,
                message: "reset Hard to HEAD".to_string(),
            },
            AuditRow {
                at: "2026-10-19T00:00:02Z".to_string(),
                actor: "human:tui".to_string(),
                capability: "git.stage".to_string(),
                tier: "write".to_string(),
                ok: true,
                dry_run: false,
                message: "staged".to_string(),
            },
        ];
        let screen = draw(&state);
        assert!(screen.contains("refused"), "{screen}");
        assert!(screen.contains("plan"), "{screen}");
        assert!(screen.contains("applied"), "{screen}");
    }

    #[test]
    fn a_failing_check_is_visible_on_the_dashboard() {
        let mut state = State::default();
        state.view = View::Dashboard;
        state.checks = vec![
            CheckRow {
                name: "layout".to_string(),
                level: "ok".to_string(),
                detail: "D:\\devd-test-home".to_string(),
            },
            CheckRow {
                name: "git".to_string(),
                level: "fail".to_string(),
                detail: "git was not found on PATH".to_string(),
            },
        ];
        let screen = draw(&state);
        assert!(screen.contains("1 failing"), "{screen}");
        assert!(screen.contains("git was not found"), "{screen}");
    }

    #[test]
    fn an_approval_prompt_shows_the_plan_the_arguments_and_the_choice() {
        let mut state = State::default();
        state.view = View::Changes;
        let mut plan = devd_core::capability::Plan::new("git.discard", "discard 1 path");
        plan = plan
            .step("git restore -- notes.txt")
            .step("uncommitted changes to notes.txt are lost")
            .destructive(true);
        state.overlay = Overlay::Approval(Pending {
            id: "git.discard".to_string(),
            plan: Some(plan),
            request: devd_core::capability::InvokeRequest {
                id: "git.discard".to_string(),
                args: serde_json::json!({ "repo": "D:/tmp/demo", "paths": ["notes.txt"] }),
                dry_run: false,
            },
        });
        let screen = draw(&state);
        assert!(screen.contains("destructive operation"), "{screen}");
        assert!(screen.contains("discard 1 path"), "{screen}");
        assert!(
            screen.contains("uncommitted changes to notes.txt are lost"),
            "{screen}"
        );
        assert!(
            screen.contains("notes.txt"),
            "the arguments are not shown: {screen}"
        );
        assert!(screen.contains("approve and run"), "{screen}");
        assert!(screen.contains("decline"), "{screen}");
    }

    #[test]
    fn an_operation_that_cannot_plan_says_so_in_the_prompt() {
        let mut state = State::default();
        state.overlay = Overlay::Approval(Pending {
            id: "git.push.force".to_string(),
            plan: None,
            request: devd_core::capability::InvokeRequest {
                id: "git.push.force".to_string(),
                args: serde_json::json!({}),
                dry_run: false,
            },
        });
        let screen = draw(&state);
        assert!(screen.contains("cannot describe itself"), "{screen}");
    }

    #[test]
    fn a_prompt_shows_the_title_and_the_typed_text() {
        let mut state = State::default();
        state.overlay = Overlay::Prompt {
            kind: PromptKind::CommitMessage,
            input: "fix the parser".to_string(),
        };
        let screen = draw(&state);
        assert!(screen.contains("Commit message"), "{screen}");
        assert!(screen.contains("fix the parser"), "{screen}");
        assert!(screen.contains("Esc cancel"), "{screen}");
    }

    #[test]
    fn the_capability_list_flags_what_needs_approval() {
        let mut state = State::default();
        state.view = View::Capabilities;
        state.capabilities = vec![
            CapRow {
                id: "git.status".to_string(),
                tier: "read".to_string(),
                summary: "Show status".to_string(),
                requires_approval: false,
                agent_safe: true,
            },
            CapRow {
                id: "git.discard".to_string(),
                tier: "destructive".to_string(),
                summary: "Discard changes".to_string(),
                requires_approval: true,
                agent_safe: true,
            },
        ];
        let screen = draw(&state);
        assert!(screen.contains("git.status"), "{screen}");
        assert!(screen.contains("git.discard"), "{screen}");
        assert!(screen.contains("approval"), "{screen}");
    }

    #[test]
    fn the_projects_panel_shows_the_path() {
        let mut state = State::default();
        state.view = View::Projects;
        state.projects = vec![RepoRow {
            name: "demo".to_string(),
            path: "D:/tmp/demo".to_string(),
            default_branch: Some("main".to_string()),
            remote_url: None,
        }];
        let screen = draw(&state);
        assert!(screen.contains("demo"), "{screen}");
        assert!(screen.contains("D:/tmp/demo"), "{screen}");
    }

    #[test]
    fn the_cursor_row_is_marked_and_the_window_follows_it() {
        let mut state = State::default();
        state.view = View::History;
        state.commits = (0..80)
            .map(|i| CommitRow {
                short: format!("hash{i:03}"),
                subject: format!("commit number {i}"),
                ..Default::default()
            })
            .collect();
        state.cursor = 75;
        let screen = draw(&state);
        assert!(
            screen.contains("commit number 75"),
            "the cursor row is off screen:\n{screen}"
        );
        assert!(
            !screen.contains("commit number 0"),
            "the window did not move:\n{screen}"
        );
    }

    #[test]
    fn a_control_character_in_a_path_cannot_reach_the_terminal() {
        // A branch name or path with an escape sequence in it is a command being
        // sent to the reader's terminal, not text. It must be neutralised.
        let mut state = State::default();
        state.view = View::Branches;
        state.branches = vec![BranchRow {
            name: "evil\u{1b}[2Jbranch".to_string(),
            current: false,
            ..Default::default()
        }];
        let screen = draw(&state);
        assert!(!screen.contains('\u{1b}'), "an escape reached the buffer");
        assert!(screen.contains("evil?[2Jbranch"), "{screen}");
    }

    #[test]
    fn the_window_keeps_the_cursor_visible() {
        assert_eq!(window(0, 0, 0, 10), 0);
        assert_eq!(window(100, 0, 0, 10), 0);
        assert_eq!(window(100, 9, 0, 10), 0);
        assert_eq!(window(100, 10, 0, 10), 1);
        assert_eq!(window(100, 50, 0, 10), 41);
        // A desired scroll is honoured as long as the cursor stays visible.
        assert_eq!(window(100, 20, 5, 10), 11);
        let first = window(100, 12, 30, 10);
        assert!(
            first <= 12 && 12 < first + 10,
            "the cursor fell outside the window: first={first}"
        );
    }

    #[test]
    fn truncate_marks_that_it_shortened() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
        // Multi-byte characters are counted as characters, not bytes.
        assert_eq!(truncate("héllo", 4), "hél…");
    }
}
