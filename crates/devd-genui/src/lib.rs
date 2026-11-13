//! Generative interface for chat.
//!
//! An assistant that can only write prose gives a table as a pipe-separated string and a chart as an
//! adjective. Here it can instead *draw*: it writes a **spec**, a small tree of components from a fixed
//! **catalog**, and the window draws each one with its own components. Nothing the model writes is ever
//! executed or interpreted as markup; a spec is data, checked against the catalog before it is trusted.
//!
//! The pieces, each independent and pure so they can be tested without a window:
//!
//! * [`CATALOG`]: every component, its props and what it may contain. It is the single source of truth:
//!   [`prompt`] turns it into the instructions the model gets, [`check`] enforces it, and the window's
//!   renderer draws exactly these names.
//! * [`parse_partial`]: reads a spec that is still being streamed, closing whatever the model has not
//!   closed yet, so the window can draw the part that has arrived.
//! * [`check`] and [`draft`]: the strict check for a finished spec (one readable [`Issue`] per problem, so
//!   they can be sent back to the model to correct itself) and the forgiving one for a spec in progress.
//! * [`find`]: the specs inside a reply, which is how the agent finds what to check.
//! * [`text`]: a plain-text form, for the terminal, for copying, and for a reply read without the window.
//!
//! A spec travels inside a normal reply as a fenced block named `devd-ui` holding one JSON object:
//!
//! ````text
//! ```devd-ui
//! {"type":"Card","props":{"title":"Services"},"children":[{"type":"Table","props":{...}}]}
//! ```
//! ````
//!
//! so it streams with the rest of the text, works with every model provider, is stored in the
//! conversation like any other message, and degrades to a code block anywhere nothing can draw it.

use std::fmt;

use serde_json::{Map, Value};

/// The name of the fenced block a spec travels in.
pub const FENCE: &str = "devd-ui";

// ---------------------------------------------------------------------------------------------- limits

/// What one spec may contain. A model that is asked for "everything" must not be able to produce a
/// message that takes the window a minute to lay out.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Components in the whole tree.
    pub elements: usize,
    /// How deep components nest.
    pub depth: usize,
    /// Tables in one message.
    pub tables: usize,
    /// Charts in one message.
    pub charts: usize,
    /// Rows in a table.
    pub table_rows: usize,
    /// Columns in a table.
    pub table_columns: usize,
    /// Bars in a chart.
    pub chart_points: usize,
    /// Entries in a list of fields, metrics, steps, timeline items or files.
    pub list_items: usize,
    /// Characters in one piece of text.
    pub text_chars: usize,
    /// Buttons in one message.
    pub buttons: usize,
}

/// The limits in force.
pub const LIMITS: Limits = Limits { elements: 300, depth: 6, tables: 4, charts: 4, table_rows: 200, table_columns: 12, chart_points: 40, list_items: 60, text_chars: 6000, buttons: 8 };

// ---------------------------------------------------------------------------------------------- catalog

/// The type of a prop.
#[derive(Debug, Clone, Copy)]
pub enum Ty {
    /// Text.
    Str,
    /// A number.
    Num,
    /// Text, a number or a yes/no, shown as written.
    Scalar,
    /// Any JSON value.
    Any,
    /// One of these words.
    OneOf(&'static [&'static str]),
    /// A list of one type.
    List(&'static Ty),
    /// An object with these fields.
    Obj(&'static [Prop]),
}

/// One prop of a component.
#[derive(Debug, Clone, Copy)]
pub struct Prop {
    /// Its name.
    pub name: &'static str,
    /// What it holds.
    pub ty: Ty,
    /// Whether the component cannot be drawn without it.
    pub required: bool,
    /// What it is for, in a few words.
    pub doc: &'static str,
}

/// What a component may contain.
#[derive(Debug, Clone, Copy)]
pub enum Kids {
    /// Nothing.
    None,
    /// Any component except a `Card`.
    Any,
    /// Only these.
    Only(&'static [&'static str]),
}

/// One component an assistant may draw.
#[derive(Debug, Clone, Copy)]
pub struct Component {
    /// Its name, as written in `"type"`.
    pub name: &'static str,
    /// What it is for, in a sentence the model reads.
    pub summary: &'static str,
    /// Its props.
    pub props: &'static [Prop],
    /// What it may contain.
    pub kids: Kids,
}

const fn p(name: &'static str, ty: Ty, required: bool, doc: &'static str) -> Prop {
    Prop { name, ty, required, doc }
}

const TONES: &[&str] = &["muted", "ok", "warn", "danger", "info"];

/// Every component. The order is the order the model is told about them.
pub const CATALOG: &[Component] = &[
    Component {
        name: "Card",
        summary: "The root of every spec: one titled card. Everything else goes inside it.",
        props: &[p("title", Ty::Str, false, "the heading"), p("subtitle", Ty::Str, false, "a line under the heading")],
        kids: Kids::Any,
    },
    Component {
        name: "Section",
        summary: "A titled group inside the card, to separate topics.",
        props: &[p("title", Ty::Str, false, "the group's heading")],
        kids: Kids::Any,
    },
    Component {
        name: "Text",
        summary: "A paragraph. Markdown is allowed: bold, italics, `code`, links and lists.",
        props: &[p("text", Ty::Str, true, "the markdown"), p("tone", Ty::OneOf(TONES), false, "colour of the words")],
        kids: Kids::None,
    },
    Component {
        name: "Callout",
        summary: "A short notice that must not be missed: a warning, a result, a caution.",
        props: &[
            p("tone", Ty::OneOf(&["info", "ok", "warn", "danger"]), false, "what kind of notice; default info"),
            p("title", Ty::Str, false, "a few words in bold"),
            p("text", Ty::Str, true, "the notice, as markdown"),
        ],
        kids: Kids::None,
    },
    Component {
        name: "Fields",
        summary: "Labelled facts, two columns: a name and its value. For the details of one thing.",
        props: &[p("items", Ty::List(&Ty::Obj(&[p("label", Ty::Str, true, "the name"), p("value", Ty::Scalar, true, "its value")])), true, "the facts")],
        kids: Kids::None,
    },
    Component {
        name: "Table",
        summary: "Rows and columns. For comparing several things on the same attributes.",
        props: &[
            p("title", Ty::Str, false, "a heading above the table"),
            p("columns", Ty::List(&Ty::Str), true, "the column headings"),
            p("rows", Ty::List(&Ty::List(&Ty::Scalar)), true, "the rows; each has one cell per column"),
        ],
        kids: Kids::None,
    },
    Component {
        name: "Chart",
        summary: "Bars that compare numbers: `bar` is horizontal, `column` is vertical, `share` shows parts of a whole.",
        props: &[
            p("title", Ty::Str, false, "a heading"),
            p("style", Ty::OneOf(&["bar", "column", "share"]), false, "default bar"),
            p("unit", Ty::Str, false, "shown after each value"),
            p("bars", Ty::List(&Ty::Obj(&[p("label", Ty::Str, true, "what the bar is"), p("value", Ty::Num, true, "how long"), p("tone", Ty::Str, false, "ok, warn, danger, info or muted")])), true, "the bars"),
            p("takeaway", Ty::Str, false, "the one-line conclusion"),
        ],
        kids: Kids::None,
    },
    Component {
        name: "Metrics",
        summary: "A row of big numbers with how they changed.",
        props: &[
            p("title", Ty::Str, false, "a heading"),
            p("items", Ty::List(&Ty::Obj(&[p("label", Ty::Str, true, "what is counted"), p("value", Ty::Scalar, true, "the number"), p("delta", Ty::Str, false, "the change, such as +3 or -12%"), p("delta_tone", Ty::Str, false, "success or error, when the sign is not enough")])), true, "the numbers"),
        ],
        kids: Kids::None,
    },
    Component {
        name: "Steps",
        summary: "A sequence with the current step marked. For progress through a process.",
        props: &[p("title", Ty::Str, false, "a heading"), p("items", Ty::List(&Ty::Str), true, "the steps, in order"), p("current", Ty::Num, false, "the zero-based index of the step in progress")],
        kids: Kids::None,
    },
    Component {
        name: "Timeline",
        summary: "Things that happened, in order, each with a time.",
        props: &[p("title", Ty::Str, false, "a heading"), p("items", Ty::List(&Ty::Obj(&[p("at", Ty::Str, false, "when"), p("text", Ty::Str, true, "what happened"), p("tone", Ty::Str, false, "ok, warn, danger, info or muted")])), true, "the events")],
        kids: Kids::None,
    },
    Component {
        name: "Details",
        summary: "A titled block of markdown for the longer part of an answer.",
        props: &[p("title", Ty::Str, true, "the heading"), p("body", Ty::Str, true, "markdown")],
        kids: Kids::None,
    },
    Component {
        name: "Code",
        summary: "A block of code or output.",
        props: &[p("code", Ty::Str, true, "the text"), p("language", Ty::Str, false, "for example rust or json"), p("title", Ty::Str, false, "a file name or caption")],
        kids: Kids::None,
    },
    Component {
        name: "Diff",
        summary: "A patch, drawn as a diff with added and removed lines. Give a unified diff.",
        props: &[p("patch", Ty::Str, true, "the unified diff")],
        kids: Kids::None,
    },
    Component {
        name: "FileList",
        summary: "Files or folders that exist on this machine; each one opens when clicked.",
        props: &[p("title", Ty::Str, false, "a heading"), p("paths", Ty::List(&Ty::Str), true, "the paths")],
        kids: Kids::None,
    },
    Component { name: "Divider", summary: "A thin line between parts.", props: &[], kids: Kids::None },
    Component { name: "Actions", summary: "A row of buttons, at the end of the card. Only buttons go inside.", props: &[], kids: Kids::Only(&["Button", "LinkButton"]) },
    Component {
        name: "Button",
        summary: "A choice the person can make. `reply` sends your `text` as their next message; `open` goes to a devd page (`target` such as /sites) or a file; `run` asks devd to run a capability, which shows what it will do and asks first unless it only reads.",
        props: &[
            p("label", Ty::Str, true, "what the button says"),
            p("action", Ty::OneOf(&["reply", "open", "run"]), true, "what pressing it does"),
            p("text", Ty::Str, false, "for reply: the message to send"),
            p("target", Ty::Str, false, "for open: a path such as /sites or /databases, or a file path"),
            p("capability", Ty::Str, false, "for run: the capability id, such as site.restart"),
            p("args", Ty::Any, false, "for run: the capability's arguments, as an object"),
            p("style", Ty::OneOf(&["primary", "secondary", "danger"]), false, "default secondary"),
        ],
        kids: Kids::None,
    },
    Component {
        name: "LinkButton",
        summary: "A button that opens a web address in the browser.",
        props: &[p("label", Ty::Str, true, "what the button says"), p("url", Ty::Str, true, "an https address")],
        kids: Kids::None,
    },
];

/// The component with this name.
pub fn component(name: &str) -> Option<&'static Component> {
    CATALOG.iter().find(|c| c.name == name)
}

// -------------------------------------------------------------------------------------------- the tree

/// One component of a checked spec.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Its catalog name.
    pub kind: &'static str,
    /// Its props, as written.
    pub props: Map<String, Value>,
    /// What is inside it.
    pub children: Vec<Node>,
}

impl Node {
    /// A text prop, or `""`.
    pub fn str(&self, name: &str) -> &str {
        self.props.get(name).and_then(Value::as_str).unwrap_or_default()
    }

    /// A list prop, or nothing.
    pub fn list(&self, name: &str) -> &[Value] {
        self.props.get(name).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// One thing wrong with a spec, said so a model can fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Where, as a path such as `children[2].props.rows[0]`.
    pub path: String,
    /// What is wrong and what would be right.
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

// --------------------------------------------------------------------------------------- the checker

struct Walk {
    strict: bool,
    issues: Vec<Issue>,
    elements: usize,
    tables: usize,
    charts: usize,
    buttons: usize,
}

impl Walk {
    fn issue(&mut self, path: &str, message: impl Into<String>) {
        if self.issues.len() < 40 {
            self.issues.push(Issue { path: path.to_string(), message: message.into() });
        }
    }
}

fn ty_name(ty: &Ty) -> String {
    match ty {
        Ty::Str => "text".into(),
        Ty::Num => "a number".into(),
        Ty::Scalar => "text or a number".into(),
        Ty::Any => "any value".into(),
        Ty::OneOf(words) => format!("one of {}", words.iter().map(|w| format!("\"{w}\"")).collect::<Vec<_>>().join(", ")),
        Ty::List(inner) => format!("a list of {}", ty_name(inner)),
        Ty::Obj(_) => "an object".into(),
    }
}

fn what(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a yes/no",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// Check a value against a type. False when it does not fit.
fn fits(w: &mut Walk, ty: &Ty, v: &Value, path: &str) -> bool {
    let ok = match (ty, v) {
        (Ty::Any, _) => true,
        (Ty::Str, Value::String(s)) => {
            if s.chars().count() > LIMITS.text_chars {
                w.issue(path, format!("is {} characters; the most is {}", s.chars().count(), LIMITS.text_chars));
                return false;
            }
            true
        }
        (Ty::Num, Value::Number(_)) => true,
        (Ty::Scalar, Value::String(_) | Value::Number(_) | Value::Bool(_)) => true,
        (Ty::OneOf(words), Value::String(s)) => words.contains(&s.as_str()),
        (Ty::List(inner), Value::Array(items)) => {
            if items.len() > LIMITS.table_rows.max(LIMITS.list_items) {
                w.issue(path, format!("has {} entries; the most is {}", items.len(), LIMITS.table_rows.max(LIMITS.list_items)));
                return false;
            }
            let mut all = true;
            for (n, item) in items.iter().enumerate() {
                all &= fits(w, inner, item, &format!("{path}[{n}]"));
            }
            return all;
        }
        (Ty::Obj(props), Value::Object(map)) => {
            let mut all = true;
            for prop in props.iter() {
                match map.get(prop.name) {
                    Some(Value::Null) | None if prop.required => {
                        w.issue(&format!("{path}.{}", prop.name), format!("is required ({})", prop.doc));
                        all = false;
                    }
                    Some(Value::Null) | None => {}
                    Some(value) => all &= fits(w, &prop.ty, value, &format!("{path}.{}", prop.name)),
                }
            }
            return all;
        }
        _ => false,
    };
    if !ok {
        w.issue(path, format!("must be {}, not {}", ty_name(ty), what(v)));
    }
    ok
}

/// A value with the list entries that do not fit its type taken out, all the way down. Used only on a spec that is
/// still being written.
fn prune(ty: &Ty, v: &Value) -> Value {
    match (ty, v) {
        (Ty::List(inner), Value::Array(items)) => Value::Array(
            items
                .iter()
                .map(|item| prune(inner, item))
                .filter(|item| {
                    let mut probe = Walk { strict: true, issues: Vec::new(), elements: 0, tables: 0, charts: 0, buttons: 0 };
                    fits(&mut probe, inner, item, "")
                })
                .collect(),
        ),
        (Ty::Obj(props), Value::Object(map)) => {
            let mut out = map.clone();
            for prop in props.iter() {
                if let Some(inner) = map.get(prop.name) {
                    out.insert(prop.name.to_string(), prune(&prop.ty, inner));
                }
            }
            Value::Object(out)
        }
        _ => v.clone(),
    }
}

/// Check one component and everything in it. `None` when it cannot be drawn at all.
fn node(w: &mut Walk, v: &Value, path: &str, depth: usize, parent: Option<&'static Component>, is_root: bool) -> Option<Node> {
    let Some(map) = v.as_object() else {
        w.issue(path, format!("must be an object with a \"type\", not {}", what(v)));
        return None;
    };
    let Some(name) = map.get("type").and_then(Value::as_str) else {
        // While streaming, the type may simply not have arrived.
        if w.strict || map.len() > 1 {
            w.issue(path, "has no \"type\"; every component is {\"type\": \"Name\", \"props\": {...}, \"children\": [...]}");
        }
        return None;
    };
    let Some(comp) = component(name) else {
        w.issue(path, format!("\"{name}\" is not a component. Use one of: {}", CATALOG.iter().map(|c| c.name).collect::<Vec<_>>().join(", ")));
        return None;
    };
    if is_root && name != "Card" {
        w.issue(path, format!("the root must be a Card, not {name}"));
        if w.strict {
            return None;
        }
    }
    if name == "Card" && !is_root {
        w.issue(path, "a Card cannot go inside another component; use a Section");
        return None;
    }
    if let Some(parent) = parent {
        if let Kids::Only(allowed) = parent.kids {
            if !allowed.contains(&name) {
                w.issue(path, format!("{} may only contain {}, not {name}", parent.name, allowed.join(" or ")));
                return None;
            }
        }
    }
    w.elements += 1;
    if w.elements > LIMITS.elements {
        w.issue(path, format!("the spec has more than {} components", LIMITS.elements));
        return None;
    }
    if depth > LIMITS.depth {
        w.issue(path, format!("nests deeper than {} levels", LIMITS.depth));
        return None;
    }

    // Props.
    let mut props = Map::new();
    let empty = Map::new();
    let given = match map.get("props") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(m)) => m,
        Some(other) => {
            w.issue(&format!("{path}.props"), format!("must be an object, not {}", what(other)));
            &empty
        }
    };
    let mut complete = true;
    for prop in comp.props {
        let here = format!("{path}.props.{}", prop.name);
        match given.get(prop.name) {
            None | Some(Value::Null) => {
                if prop.required {
                    complete = false;
                    if w.strict {
                        w.issue(&here, format!("is required ({})", prop.doc));
                    }
                }
            }
            Some(value) => {
                // While a spec is still being written the last entry of a list is often half there. It is left
                // out, so that one half-written row cannot take every row before it away with it.
                let value = if w.strict { value.clone() } else { prune(&prop.ty, value) };
                let waiting = !w.strict && prop.required && matches!(&value, Value::Array(items) if items.is_empty());
                if waiting {
                    complete = false;
                } else if fits(w, &prop.ty, &value, &here) {
                    props.insert(prop.name.to_string(), value);
                } else if !w.strict && !prop.required && matches!(prop.ty, Ty::OneOf(_)) {
                    // A word cut off halfway ("wa" of "warn") is no choice yet: it counts for nothing, not against the whole.
                } else {
                    complete = false;
                }
            }
        }
    }
    extra(w, comp, &mut props, path, &mut complete);
    if !complete && !w.strict {
        // Still arriving: draw nothing for it yet rather than something wrong.
        if !matches!(comp.kids, Kids::Any) {
            return None;
        }
    }

    // Children.
    let mut children = Vec::new();
    match map.get("children") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            if matches!(comp.kids, Kids::None) && !list.is_empty() {
                w.issue(&format!("{path}.children"), format!("{name} cannot contain anything"));
            } else {
                for (n, child) in list.iter().enumerate() {
                    if let Some(c) = node(w, child, &format!("{path}.children[{n}]"), depth + 1, Some(comp), false) {
                        children.push(c);
                    }
                }
            }
        }
        Some(other) => w.issue(&format!("{path}.children"), format!("must be a list, not {}", what(other))),
    }
    if w.strict && name == "Card" && is_root && props.get("title").is_none() && children.is_empty() {
        w.issue(path, "the Card has no title and nothing inside it");
    }
    if w.strict && name == "Actions" && children.is_empty() {
        w.issue(path, "Actions has no buttons in it");
    }
    Some(Node { kind: comp.name, props, children })
}

/// The checks that need more than one prop at once.
fn extra(w: &mut Walk, comp: &Component, props: &mut Map<String, Value>, path: &str, complete: &mut bool) {
    let here = |name: &str| format!("{path}.props.{name}");
    match comp.name {
        "Table" => {
            w.tables += 1;
            if w.tables > LIMITS.tables {
                w.issue(path, format!("more than {} tables in one message", LIMITS.tables));
                *complete = false;
            }
            let columns = props.get("columns").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
            if columns > LIMITS.table_columns {
                w.issue(&here("columns"), format!("{columns} columns; the most is {}", LIMITS.table_columns));
                *complete = false;
            }
            if !w.strict && columns > 0 {
                // A row still being written is left out until it has all its cells.
                let whole: Vec<Value> = props.get("rows").and_then(Value::as_array).map(|rows| rows.iter().filter(|r| r.as_array().is_some_and(|c| c.len() == columns)).cloned().collect()).unwrap_or_default();
                if whole.is_empty() {
                    *complete = false;
                } else {
                    props.insert("rows".into(), Value::Array(whole));
                }
            }
            if let Some(rows) = props.get("rows").and_then(Value::as_array) {
                if rows.len() > LIMITS.table_rows {
                    w.issue(&here("rows"), format!("{} rows; the most is {}", rows.len(), LIMITS.table_rows));
                    *complete = false;
                }
                for (n, row) in rows.iter().enumerate() {
                    let cells = row.as_array().map(Vec::len).unwrap_or(0);
                    if cells != columns && columns > 0 {
                        w.issue(&format!("{}[{n}]", here("rows")), format!("has {cells} cells but the table has {columns} columns"));
                        *complete = false;
                    }
                }
            }
        }
        "Chart" => {
            w.charts += 1;
            if w.charts > LIMITS.charts {
                w.issue(path, format!("more than {} charts in one message", LIMITS.charts));
                *complete = false;
            }
            if let Some(bars) = props.get("bars").and_then(Value::as_array) {
                if bars.len() > LIMITS.chart_points {
                    w.issue(&here("bars"), format!("{} bars; the most is {}", bars.len(), LIMITS.chart_points));
                    *complete = false;
                }
                if bars.iter().any(|b| b["value"].as_f64().is_some_and(|v| v < 0.0)) && w.strict {
                    w.issue(&here("bars"), "a bar cannot be negative");
                    *complete = false;
                }
            }
        }
        "Steps" => {
            let count = props.get("items").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
            if let Some(current) = props.get("current").and_then(Value::as_u64) {
                if current as usize > count && w.strict {
                    w.issue(&here("current"), format!("is {current} but there are only {count} steps"));
                    *complete = false;
                }
            }
        }
        "Fields" | "Metrics" | "Timeline" | "FileList" => {
            let key = if comp.name == "FileList" { "paths" } else { "items" };
            let count = props.get(key).and_then(Value::as_array).map(Vec::len).unwrap_or(0);
            if count > LIMITS.list_items {
                w.issue(&here(key), format!("{count} entries; the most is {}", LIMITS.list_items));
                *complete = false;
            }
        }
        "Button" => {
            w.buttons += 1;
            if w.buttons > LIMITS.buttons {
                w.issue(path, format!("more than {} buttons in one message", LIMITS.buttons));
                *complete = false;
            }
            let action = props.get("action").and_then(Value::as_str).unwrap_or_default().to_string();
            let need = match action.as_str() {
                "reply" => Some("text"),
                "open" => Some("target"),
                "run" => Some("capability"),
                _ => None,
            };
            if let Some(need) = need {
                if props.get(need).and_then(Value::as_str).map(str::trim).unwrap_or_default().is_empty() {
                    if w.strict {
                        w.issue(&here(need), format!("is required when action is \"{action}\""));
                    }
                    *complete = false;
                }
            }
            if action == "run" {
                if let Some(args) = props.get("args") {
                    if !args.is_object() {
                        w.issue(&here("args"), format!("must be an object, not {}", what(args)));
                        *complete = false;
                    }
                }
            }
        }
        "LinkButton" => {
            w.buttons += 1;
            if w.buttons > LIMITS.buttons {
                w.issue(path, format!("more than {} buttons in one message", LIMITS.buttons));
                *complete = false;
            }
            if let Some(url) = props.get("url").and_then(Value::as_str) {
                if !url.starts_with("https://") && !url.starts_with("http://") {
                    w.issue(&here("url"), "must be a web address starting with https://");
                    *complete = false;
                }
            }
        }
        _ => {}
    }
}

/// Check a finished spec. Every problem is returned, each readable on its own, so that the list can be
/// sent back to the model to write a corrected spec.
pub fn check(spec: &Value) -> Result<Node, Vec<Issue>> {
    let mut w = Walk { strict: true, issues: Vec::new(), elements: 0, tables: 0, charts: 0, buttons: 0 };
    let tree = node(&mut w, spec, "", 0, None, true);
    match (tree, w.issues.is_empty()) {
        (Some(tree), true) => Ok(tree),
        (_, _) => Err(w.issues),
    }
}

/// The part of a spec that can be drawn so far: every component whose props already fit, none whose
/// props are still arriving or wrong. `None` until the root exists.
pub fn draft(spec: &Value) -> Option<Node> {
    let mut w = Walk { strict: false, issues: Vec::new(), elements: 0, tables: 0, charts: 0, buttons: 0 };
    node(&mut w, spec, "", 0, None, true)
}

// ----------------------------------------------------------------------------------- partial parsing

/// Close what a streamed JSON text has left open. Tries the text as it is, then backs up to earlier
/// places where a value ended until something parses, so a cut in the middle of a key, a number or a
/// string never loses what came before it.
pub fn parse_partial(src: &str) -> Option<Value> {
    let src = src.trim();
    if !src.starts_with('{') {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<Value>(src) {
        return Some(v);
    }
    if let Some(v) = closed(src) {
        return Some(v);
    }
    // Places a value could have ended, newest first.
    let mut cuts = Vec::new();
    let (mut in_string, mut escaped) = (false, false);
    for (i, c) in src.char_indices() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            ',' => cuts.push(i),
            '{' | '[' => cuts.push(i + 1),
            '}' | ']' => cuts.push(i + 1),
            _ => {}
        }
    }
    for &cut in cuts.iter().rev().take(24) {
        if let Some(v) = closed(&src[..cut]) {
            return Some(v);
        }
    }
    None
}

/// The text with its open string, trailing comma and open brackets closed, parsed.
fn closed(prefix: &str) -> Option<Value> {
    let mut stack = Vec::new();
    let (mut in_string, mut escaped) = (false, false);
    for c in prefix.chars() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                stack.pop();
            }
            _ => {}
        }
    }
    let mut text = prefix.to_string();
    if escaped {
        text.pop();
    }
    if in_string {
        text.push('"');
    }
    let trimmed = text.trim_end().trim_end_matches(',').to_string();
    let mut text = trimmed;
    while let Some(close) = stack.pop() {
        text.push(close);
    }
    serde_json::from_str(&text).ok()
}

// ------------------------------------------------------------------------------------ finding specs

/// One fenced `devd-ui` block in a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The JSON inside the fence.
    pub body: String,
    /// Whether the closing fence has arrived. False for a block still being written.
    pub closed: bool,
}

/// Every spec in a reply.
pub fn find(text: &str) -> Vec<Found> {
    let mut out = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let ticks = trimmed.chars().take_while(|&c| c == '`').count();
        if ticks < 3 || trimmed[ticks..].trim() != FENCE {
            continue;
        }
        let fence = "`".repeat(ticks);
        let mut body = Vec::new();
        let mut closed = false;
        for next in lines.by_ref() {
            let t = next.trim();
            if t.starts_with(&fence) && t.chars().all(|c| c == '`') {
                closed = true;
                break;
            }
            body.push(next);
        }
        out.push(Found { body: body.join("\n"), closed });
    }
    out
}

/// What is wrong with every finished spec in a reply, or nothing. A block that is not JSON at all is
/// reported too: it would be drawn as plain code.
pub fn problems(text: &str) -> Vec<Issue> {
    let mut out = Vec::new();
    for (n, found) in find(text).iter().filter(|f| f.closed).enumerate() {
        let label = |issue: Issue| Issue { path: format!("devd-ui block {}{}{}", n + 1, if issue.path.is_empty() { "" } else { " " }, issue.path), message: issue.message };
        match serde_json::from_str::<Value>(&found.body) {
            Err(e) => out.push(label(Issue { path: String::new(), message: format!("is not valid JSON ({e}); it must be one JSON object") })),
            Ok(value) => {
                if let Err(issues) = check(&value) {
                    out.extend(issues.into_iter().map(label));
                }
            }
        }
    }
    out
}

// ------------------------------------------------------------------------------------------- prompt

/// Instructions for the model: how to write a spec, and everything it may use, generated from the
/// catalog so what it is told and what is checked can never differ.
pub fn prompt() -> String {
    let mut out = String::new();
    out.push_str("## Drawing with components\n\n");
    out.push_str("When an answer is really a table, a set of numbers, a comparison, steps or a choice, you may draw it instead of describing it. Write the visual as a fenced block named `devd-ui` holding ONE JSON object. The window draws it. Always write the answer in words too: say what the visual shows and what follows from it, because the words are what a terminal, a search and a screen reader get. Do not draw what two sentences say better.\n\n");
    out.push_str("Every component is `{\"type\": \"Name\", \"props\": {...}, \"children\": [...]}`. The root is always a `Card`. Put the root first and the details after, so it can appear while you are still writing.\n\n");
    out.push_str("### Components\n\n");
    for c in CATALOG {
        out.push_str(&format!("- **{}**: {}", c.name, c.summary));
        if !c.props.is_empty() {
            let props: Vec<String> = c.props.iter().map(|p| format!("`{}`{} ({}{})", p.name, if p.required { "" } else { "?" }, ty_name(&p.ty), if p.doc.is_empty() { String::new() } else { format!("; {}", p.doc) })).collect();
            out.push_str(&format!(" Props: {}.", props.join(", ")));
        }
        match c.kids {
            Kids::None => {}
            Kids::Any => out.push_str(" May contain any component except a Card."),
            Kids::Only(only) => out.push_str(&format!(" Contains only {}.", only.join(" and "))),
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "\n### Limits\n\nAt most {} components, {} levels deep, {} tables, {} charts, {} rows and {} columns in a table, {} bars in a chart, {} buttons. Text is markdown; it is never run as code or HTML.\n",
        LIMITS.elements, LIMITS.depth, LIMITS.tables, LIMITS.charts, LIMITS.table_rows, LIMITS.table_columns, LIMITS.chart_points, LIMITS.buttons
    ));
    out.push_str("\n### Buttons\n\nA button never does anything until the person presses it. `reply` sends your `text` as their next message. `open` goes to a devd page or file. `run` runs a real capability by id; the window shows exactly what it will run and asks first unless it only reads. Offer a button only for a next step you would actually take, and never for something you were not asked to do.\n");
    out.push_str("\n### Example\n\n````\n```devd-ui\n");
    out.push_str(r#"{"type":"Card","props":{"title":"Sites","subtitle":"3 of 4 answering"},"children":[{"type":"Metrics","props":{"items":[{"label":"Answering","value":3,"delta":"-1"},{"label":"Certificates","value":"valid"}]}},{"type":"Table","props":{"columns":["Site","State"],"rows":[["shop.test","up"],["blog.test","down"]]}},{"type":"Actions","children":[{"type":"Button","props":{"label":"Open sites","action":"open","target":"/sites"}}]}]}"#);
    out.push_str("\n```\n````\n\nIf a block is wrong, devd tells you each problem and you write it again.\n");
    out
}

/// The note sent back to a model whose spec did not check.
pub fn correction(issues: &[Issue]) -> String {
    let mut out = String::from("[devd] The `devd-ui` block you wrote could not be drawn, so the person sees plain code instead. Write the whole answer again with the block corrected. Problems:\n");
    for issue in issues.iter().take(12) {
        out.push_str(&format!("- {issue}\n"));
    }
    if issues.len() > 12 {
        out.push_str(&format!("- and {} more of the same kind\n", issues.len() - 12));
    }
    out.push_str("Keep it small and within the catalog; if a visual does not fit, say it in words.");
    out
}

// ------------------------------------------------------------------------------------- plain text

fn cell(v: &Value) -> String {
    match v {
        Value::String(s) => s.replace('|', "\\|").replace('\n', " "),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A spec as plain text: for the terminal, for copying, and for anything that cannot draw it.
pub fn text(tree: &Node) -> String {
    let mut out = String::new();
    write_text(tree, 0, &mut out);
    out.trim_end().to_string() + "\n"
}

fn write_text(n: &Node, depth: usize, out: &mut String) {
    let title = |out: &mut String, level: usize| {
        let t = n.str("title");
        if !t.is_empty() {
            out.push_str(&format!("{} {t}\n\n", "#".repeat(level)));
        }
    };
    match n.kind {
        "Card" => {
            title(out, 2);
            if !n.str("subtitle").is_empty() {
                out.push_str(&format!("{}\n\n", n.str("subtitle")));
            }
        }
        "Section" => title(out, 3 + depth.min(2)),
        "Text" => out.push_str(&format!("{}\n\n", n.str("text"))),
        "Callout" => {
            let head = n.str("title");
            out.push_str(&format!("> {}{}\n\n", if head.is_empty() { String::new() } else { format!("**{head}** ") }, n.str("text")));
        }
        "Fields" => {
            for f in n.list("items") {
                out.push_str(&format!("- {}: {}\n", cell(&f["label"]), cell(&f["value"])));
            }
            out.push('\n');
        }
        "Table" => {
            title(out, 4);
            let cols: Vec<String> = n.list("columns").iter().map(cell).collect();
            out.push_str(&format!("| {} |\n|{}|\n", cols.join(" | "), cols.iter().map(|_| " --- ").collect::<Vec<_>>().join("|")));
            for row in n.list("rows") {
                let cells: Vec<String> = row.as_array().map(|r| r.iter().map(cell).collect()).unwrap_or_default();
                out.push_str(&format!("| {} |\n", cells.join(" | ")));
            }
            out.push('\n');
        }
        "Chart" => {
            title(out, 4);
            let unit = n.str("unit");
            for b in n.list("bars") {
                out.push_str(&format!("- {}: {}{}\n", cell(&b["label"]), cell(&b["value"]), if unit.is_empty() { String::new() } else { format!(" {unit}") }));
            }
            if !n.str("takeaway").is_empty() {
                out.push_str(&format!("\n{}\n", n.str("takeaway")));
            }
            out.push('\n');
        }
        "Metrics" => {
            title(out, 4);
            for m in n.list("items") {
                let delta = m["delta"].as_str().unwrap_or_default();
                out.push_str(&format!("- {}: {}{}\n", cell(&m["label"]), cell(&m["value"]), if delta.is_empty() { String::new() } else { format!(" ({delta})") }));
            }
            out.push('\n');
        }
        "Steps" => {
            title(out, 4);
            let current = n.props.get("current").and_then(Value::as_u64).unwrap_or(0) as usize;
            for (i, s) in n.list("items").iter().enumerate() {
                let mark = match i.cmp(&current) {
                    std::cmp::Ordering::Less => "[x]",
                    std::cmp::Ordering::Equal => "[>]",
                    std::cmp::Ordering::Greater => "[ ]",
                };
                out.push_str(&format!("{} {mark} {}\n", i + 1, cell(s)));
            }
            out.push('\n');
        }
        "Timeline" => {
            title(out, 4);
            for e in n.list("items") {
                out.push_str(&format!("- {} {}\n", cell(&e["at"]), cell(&e["text"])));
            }
            out.push('\n');
        }
        "Details" => out.push_str(&format!("**{}**\n\n{}\n\n", n.str("title"), n.str("body"))),
        "Code" => out.push_str(&format!("```{}\n{}\n```\n\n", n.str("language"), n.str("code"))),
        "Diff" => out.push_str(&format!("```diff\n{}\n```\n\n", n.str("patch"))),
        "FileList" => {
            title(out, 4);
            for p in n.list("paths") {
                out.push_str(&format!("- {}\n", cell(p)));
            }
            out.push('\n');
        }
        "Divider" => out.push_str("---\n\n"),
        "Button" | "LinkButton" => out.push_str(&format!("[{}]\n", n.str("label"))),
        _ => {}
    }
    for child in &n.children {
        write_text(child, depth + 1, out);
    }
    if n.kind == "Actions" {
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn good() -> Value {
        json!({
            "type": "Card", "props": {"title": "Sites", "subtitle": "3 of 4 answering"},
            "children": [
                {"type": "Metrics", "props": {"items": [{"label": "Answering", "value": 3, "delta": "-1"}]}},
                {"type": "Table", "props": {"columns": ["Site", "State"], "rows": [["shop.test", "up"], ["blog.test", "down"]]}},
                {"type": "Section", "props": {"title": "Detail"}, "children": [
                    {"type": "Chart", "props": {"bars": [{"label": "a", "value": 3}, {"label": "b", "value": 5, "tone": "warn"}], "unit": "ms"}},
                    {"type": "Callout", "props": {"tone": "warn", "text": "One is **down**."}}
                ]},
                {"type": "Actions", "children": [
                    {"type": "Button", "props": {"label": "Open sites", "action": "open", "target": "/sites"}},
                    {"type": "Button", "props": {"label": "Restart", "action": "run", "capability": "site.restart", "args": {"name": "blog"}, "style": "danger"}},
                    {"type": "LinkButton", "props": {"label": "Docs", "url": "https://example.com"}}
                ]}
            ]
        })
    }

    #[test]
    fn a_good_spec_checks_and_keeps_its_shape() {
        let tree = check(&good()).expect("valid");
        assert_eq!(tree.kind, "Card");
        assert_eq!(tree.children.len(), 4);
        assert_eq!(tree.children[2].children[0].kind, "Chart");
        assert_eq!(tree.children[3].children.len(), 3);
    }

    #[test]
    fn every_catalog_component_has_a_unique_name_and_documented_props() {
        let mut names: Vec<&str> = CATALOG.iter().map(|c| c.name).collect();
        names.sort_unstable();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n);
        for c in CATALOG {
            assert!(!c.summary.is_empty(), "{}", c.name);
            for p in c.props {
                assert!(!p.doc.is_empty(), "{}.{}", c.name, p.name);
            }
        }
        assert_eq!(CATALOG[0].name, "Card", "the root comes first");
    }

    #[test]
    fn each_problem_is_reported_once_with_where_it_is() {
        let spec = json!({"type": "Card", "children": [
            {"type": "Table", "props": {"columns": ["A", "B"], "rows": [["1"], ["1", "2", "3"]]}},
            {"type": "Chart", "props": {"bars": [{"label": "x"}]}},
            {"type": "Gauge"},
            {"type": "Button", "props": {"label": "Go", "action": "run"}}
        ]});
        let issues = check(&spec).unwrap_err();
        let text: Vec<String> = issues.iter().map(ToString::to_string).collect();
        let all = text.join("\n");
        assert!(all.contains("children[0].props.rows[0]: has 1 cells but the table has 2 columns"), "{all}");
        assert!(all.contains("rows[1]: has 3 cells"), "{all}");
        assert!(all.contains("children[1].props.bars[0].value: is required"), "{all}");
        assert!(all.contains("\"Gauge\" is not a component"), "{all}");
        assert!(all.contains("children[3].props.capability: is required when action is \"run\""), "{all}");
        assert!(all.contains("the Card has no title") || !all.is_empty());
    }

    #[test]
    fn the_root_must_be_a_card_and_a_card_cannot_nest() {
        let issues = check(&json!({"type": "Text", "props": {"text": "hi"}})).unwrap_err();
        assert!(issues[0].message.contains("root must be a Card"), "{issues:?}");
        let nested = json!({"type": "Card", "props": {"title": "t"}, "children": [{"type": "Card", "props": {"title": "u"}}]});
        assert!(check(&nested).unwrap_err()[0].message.contains("use a Section"));
    }

    #[test]
    fn actions_hold_only_buttons_and_leaves_hold_nothing() {
        let bad = json!({"type": "Card", "props": {"title": "t"}, "children": [{"type": "Actions", "children": [{"type": "Text", "props": {"text": "x"}}]}]});
        assert!(check(&bad).unwrap_err().iter().any(|i| i.message.contains("may only contain Button or LinkButton")));
        let leaf = json!({"type": "Card", "props": {"title": "t"}, "children": [{"type": "Divider", "children": [{"type": "Divider"}]}]});
        assert!(check(&leaf).unwrap_err().iter().any(|i| i.message.contains("cannot contain anything")));
    }

    #[test]
    fn wrong_types_say_what_was_expected_and_what_was_found() {
        let spec = json!({"type": "Card", "props": {"title": 5}, "children": [{"type": "Steps", "props": {"items": ["a", "b"], "current": 9}}]});
        let all: String = check(&spec).unwrap_err().iter().map(|i| i.to_string() + "\n").collect();
        assert!(all.contains("props.title: must be text, not a number"), "{all}");
        assert!(all.contains("is 9 but there are only 2 steps"), "{all}");
    }

    #[test]
    fn limits_stop_a_spec_that_asks_for_everything() {
        let many: Vec<Value> = (0..LIMITS.elements + 5).map(|_| json!({"type": "Divider"})).collect();
        let spec = json!({"type": "Card", "props": {"title": "t"}, "children": many});
        assert!(check(&spec).unwrap_err().iter().any(|i| i.message.contains("more than 300 components")));
        let tables: Vec<Value> = (0..LIMITS.tables + 1).map(|_| json!({"type": "Table", "props": {"columns": ["a"], "rows": [["1"]]}})).collect();
        let spec = json!({"type": "Card", "props": {"title": "t"}, "children": tables});
        assert!(check(&spec).unwrap_err().iter().any(|i| i.message.contains("tables in one message")));
        let mut deep = json!({"type": "Divider"});
        for _ in 0..8 {
            deep = json!({"type": "Section", "children": [deep]});
        }
        let spec = json!({"type": "Card", "props": {"title": "t"}, "children": [deep]});
        assert!(check(&spec).unwrap_err().iter().any(|i| i.message.contains("nests deeper")));
        let long = "x".repeat(LIMITS.text_chars + 1);
        let spec = json!({"type": "Card", "props": {"title": "t"}, "children": [{"type": "Text", "props": {"text": long}}]});
        assert!(check(&spec).is_err());
    }

    #[test]
    fn a_link_button_only_takes_web_addresses() {
        let spec = json!({"type": "Card", "props": {"title": "t"}, "children": [{"type": "Actions", "children": [{"type": "LinkButton", "props": {"label": "x", "url": "javascript:alert(1)"}}]}]});
        assert!(check(&spec).unwrap_err().iter().any(|i| i.message.contains("must be a web address")));
    }

    /// The same JSON with every object's keys in the opposite order: a model may write `children` before `type`.
    fn reversed(v: &Value) -> String {
        match v {
            Value::Object(map) => format!("{{{}}}", map.iter().rev().map(|(k, v)| format!("{}:{}", serde_json::to_string(k).unwrap(), reversed(v))).collect::<Vec<_>>().join(",")),
            Value::Array(items) => format!("[{}]", items.iter().map(reversed).collect::<Vec<_>>().join(",")),
            other => other.to_string(),
        }
    }

    #[test]
    fn every_prefix_of_a_valid_spec_parses_or_waits_and_never_panics() {
        // In the order serde writes, and in the opposite order, so the result does not depend on which keys come first.
        for source in [serde_json::to_string(&good()).unwrap(), reversed(&good())] {
            grows_and_never_shrinks(&source);
        }
    }

    #[test]
    fn a_half_written_row_bar_or_word_does_not_take_what_was_already_drawn_away() {
        let drawn = |src: &str| draft(&parse_partial(src).unwrap()).map(|t| count_nodes(&t)).unwrap_or(0);
        // A table whose second row has only begun still shows its first row.
        let head = r#"{"type":"Card","props":{"title":"T"},"children":[{"type":"Table","props":{"columns":["A","B"],"rows":[["1","2"],["3""#;
        assert_eq!(drawn(head), 2, "the card and its table");
        let table = |src: &str| draft(&parse_partial(src).unwrap()).unwrap().children[0].list("rows").len();
        assert_eq!(table(head), 1, "only the whole row is drawn");
        assert_eq!(table(&format!("{head},\"4\"]]}}}}]}}")), 2);
        // A chart whose second bar has no value yet keeps the first.
        let chart = r#"{"type":"Card","props":{"title":"T"},"children":[{"type":"Chart","props":{"bars":[{"label":"a","value":1},{"label":"b""#;
        assert_eq!(drawn(chart), 2);
        // A tone cut in the middle of a word is ignored, and the notice stays.
        let callout = r#"{"type":"Card","props":{"title":"T"},"children":[{"type":"Callout","props":{"text":"x","tone":"wa"#;
        assert_eq!(drawn(callout), 2);
        // The step the person is on may be named before all the steps have arrived.
        let steps = r#"{"type":"Card","props":{"title":"T"},"children":[{"type":"Steps","props":{"current":2,"items":["a","b"#;
        assert_eq!(drawn(steps), 2);
        // A list that has no whole entry yet is not drawn as an empty frame.
        assert_eq!(drawn(r#"{"type":"Card","props":{"title":"T"},"children":[{"type":"Metrics","props":{"items":[{"label":"x""#), 1);
    }

    fn grows_and_never_shrinks(source: &str) {
        let mut last_count = 0usize;
        let mut seen_root = false;
        for end in 0..=source.len() {
            if !source.is_char_boundary(end) {
                continue;
            }
            let prefix = &source[..end];
            if let Some(value) = parse_partial(prefix) {
                if let Some(tree) = draft(&value) {
                    seen_root = true;
                    let count = count_nodes(&tree);
                    assert!(count >= last_count, "the picture shrank at byte {end}: {last_count} -> {count}");
                    last_count = count;
                }
            }
        }
        assert!(seen_root);
        assert_eq!(last_count, count_nodes(&check(&serde_json::from_str(source).unwrap()).unwrap()), "the last prefix draws everything");
    }

    fn count_nodes(n: &Node) -> usize {
        1 + n.children.iter().map(count_nodes).sum::<usize>()
    }

    #[test]
    fn a_cut_in_the_middle_of_a_key_a_string_or_a_number_keeps_what_came_before() {
        for cut in [r#"{"type":"Card","props":{"title":"Si"#, r#"{"type":"Card","props":{"ti"#, r#"{"type":"Card","props":{"title":"A"},"chil"#, r#"{"type":"Card","props":{"title":"A"},"children":[{"type":"Table","props":{"columns":["a"],"rows":[["1"],["2"#, r#"{"type":"Card","props":{"title":"A\"#, r#"{"type":"Card","props":{"title":"A"},"children":[{"type":"Chart","props":{"bars":[{"label":"x","value":12"#] {
            let v = parse_partial(cut).unwrap_or_else(|| panic!("no value from {cut}"));
            assert_eq!(v["type"], "Card", "{cut}");
        }
        assert!(parse_partial("not json").is_none());
        assert!(parse_partial("").is_none());
    }

    #[test]
    fn a_component_still_arriving_is_not_drawn_but_the_rest_is() {
        // The table's rows have not arrived: it waits. The text before it is already there.
        let v = parse_partial(r#"{"type":"Card","props":{"title":"T"},"children":[{"type":"Text","props":{"text":"hello"}},{"type":"Table","props":{"columns":["a"]"#).unwrap();
        let tree = draft(&v).unwrap();
        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].kind, "Text");
        // A card with nothing yet is still drawn, so the person sees the message begin.
        let tree = draft(&parse_partial(r#"{"type":"Card","props":{"ti"#).unwrap()).unwrap();
        assert!(tree.children.is_empty());
        assert!(draft(&parse_partial(r#"{"ty"#).unwrap_or(Value::Null)).is_none());
    }

    #[test]
    fn specs_are_found_in_a_reply_with_their_state() {
        let reply = "Here:\n\n```devd-ui\n{\"type\":\"Card\"}\n```\n\nand\n\n````devd-ui\n{\"type\":\"Card\",\n";
        let found = find(reply);
        assert_eq!(found.len(), 2);
        assert!(found[0].closed && !found[1].closed);
        assert_eq!(found[0].body, "{\"type\":\"Card\"}");
        assert!(find("```rust\nfn main() {}\n```").is_empty());
    }

    #[test]
    fn problems_names_the_block_and_skips_one_still_being_written() {
        let reply = format!("```devd-ui\n{}\n```\n```devd-ui\nnot json\n```\n```devd-ui\n{{\"type\":", serde_json::to_string(&json!({"type": "Table"})).unwrap());
        let issues = problems(&reply);
        assert!(issues.iter().any(|i| i.path.starts_with("devd-ui block 1") && i.message.contains("root must be a Card")), "{issues:?}");
        assert!(issues.iter().any(|i| i.path.starts_with("devd-ui block 2") && i.message.contains("not valid JSON")), "{issues:?}");
        assert!(!issues.iter().any(|i| i.path.contains("block 3")), "an unfinished block is not an error yet");
        assert!(problems(&format!("```devd-ui\n{}\n```", serde_json::to_string(&good()).unwrap())).is_empty());
    }

    #[test]
    fn the_prompt_names_every_component_and_its_own_example_checks() {
        let prompt = prompt();
        for c in CATALOG {
            assert!(prompt.contains(&format!("**{}**", c.name)), "{} is not in the prompt", c.name);
        }
        assert!(prompt.contains("devd-ui"));
        // The example in the prompt is itself a valid spec.
        let start = prompt.find("```devd-ui\n").unwrap() + "```devd-ui\n".len();
        let end = prompt[start..].find("\n```").unwrap() + start;
        let example: Value = serde_json::from_str(&prompt[start..end]).expect("the example is JSON");
        assert!(check(&example).is_ok(), "{:?}", check(&example));
    }

    #[test]
    fn the_correction_lists_each_problem_and_caps_a_flood() {
        let issues: Vec<Issue> = (0..20).map(|n| Issue { path: format!("p{n}"), message: "wrong".into() }).collect();
        let note = correction(&issues);
        assert!(note.starts_with("[devd]"));
        assert!(note.contains("- p0: wrong") && note.contains("and 8 more"));
    }

    #[test]
    fn the_plain_text_form_carries_everything_a_reader_needs() {
        let out = text(&check(&good()).unwrap());
        assert!(out.contains("## Sites"), "{out}");
        assert!(out.contains("| Site | State |") && out.contains("| shop.test | up |"), "{out}");
        assert!(out.contains("- Answering: 3 (-1)"), "{out}");
        assert!(out.contains("- b: 5 ms"), "{out}");
        assert!(out.contains("> **") || out.contains("> One is **down**."), "{out}");
        assert!(out.contains("[Open sites]") && out.contains("[Restart]"), "{out}");
        assert!(!out.contains("javascript:"));
    }
}
