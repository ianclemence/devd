//! Markdown, read into blocks the window draws itself.
//!
//! An agent writes Markdown; the web interface turned it into HTML and trusted a
//! sanitiser. There is no HTML here at all. The text is parsed into a small tree
//! of [`Block`]s and [`Inline`]s, and the window draws each with its own
//! components, so there is nothing to sanitise: a `<script>` in a reply is the
//! eight characters `<script>`.
//!
//! The parser covers what agents actually write (CommonMark's common core plus
//! tables, task lists and strikethrough) and is deliberately forgiving, because
//! it also runs on a reply that is still being streamed: an unclosed code fence
//! is a code block to the end of what has arrived, and an unclosed `**` is
//! literal text until its partner shows up.
//!
//! A fenced block whose language is `chart`, `metrics`, `steps`, `timeline` or
//! `details` and whose body is valid JSON becomes a [`Block::Widget`]; anything
//! else stays a code block, so bad data is shown rather than dropped.

use serde_json::Value;

/// A piece of a document that stands on its own line or lines.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// `# Title` to `###### Title`.
    Heading { level: u8, text: Vec<Inline> },
    /// Running text.
    Paragraph(Vec<Inline>),
    /// A fenced or indented code block.
    Code { lang: String, text: String },
    /// `> quoted`.
    Quote(Vec<Block>),
    /// A bulleted or numbered list.
    List { ordered: bool, start: u64, items: Vec<Item> },
    /// A table with a header row.
    Table { head: Vec<Vec<Inline>>, align: Vec<Align>, rows: Vec<Vec<Vec<Inline>>> },
    /// `---`.
    Rule,
    /// A structured block the window draws as a visual.
    Widget { kind: WidgetKind, data: Value },
    /// A spec the assistant wrote, drawn from the catalog in `devd-genui`.
    Ui { tree: Option<devd_genui::Node>, state: UiState },
}

/// How far a `devd-ui` block has got.
#[derive(Debug, Clone, PartialEq)]
pub enum UiState {
    /// Still being written: what has arrived is drawn, and the rest is on its way.
    Writing,
    /// Finished and valid.
    Done,
    /// Finished but wrong. What could be drawn is drawn, and the problems are kept.
    Broken(Vec<devd_genui::Issue>),
}

/// One entry of a list.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// `Some` for a task-list entry (`- [ ]`, `- [x]`).
    pub checked: Option<bool>,
    /// What the entry contains.
    pub blocks: Vec<Block>,
}

/// How a table column lines up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// The default.
    Start,
    /// `:---:`.
    Center,
    /// `---:`, which is what a column of numbers wants.
    End,
}

/// The visuals a fenced block can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetKind {
    /// Labelled bars.
    Chart,
    /// A row of numbers with their change.
    Metrics,
    /// A sequence with the current step marked.
    Steps,
    /// Things that happened, in order.
    Timeline,
    /// A folded section with a title.
    Details,
}

/// A piece of text inside a block.
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    /// Plain text.
    Text(String),
    /// `**strong**`.
    Strong(Vec<Inline>),
    /// `*emphasis*`.
    Emphasis(Vec<Inline>),
    /// `~~struck~~`.
    Strike(Vec<Inline>),
    /// `` `code` ``.
    Code(String),
    /// `[text](url)` or a bare `https://…`.
    Link { text: Vec<Inline>, url: String },
    /// A line break inside a paragraph.
    Break,
}

/// A run of text with one style, which is what a text renderer consumes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    /// The characters.
    pub text: String,
    /// Bold.
    pub strong: bool,
    /// Italic.
    pub emphasis: bool,
    /// Struck through.
    pub strike: bool,
    /// Monospaced, on a tinted background.
    pub code: bool,
    /// Where it leads, when it is a link.
    pub link: Option<String>,
}

/// Parse a document.
pub fn parse(source: &str) -> Vec<Block> {
    let lines: Vec<&str> = source.lines().collect();
    blocks(&lines)
}

/// Flatten inline content into styled runs, in reading order.
pub fn spans(inlines: &[Inline]) -> Vec<Span> {
    fn walk(inlines: &[Inline], style: &Span, out: &mut Vec<Span>) {
        for inline in inlines {
            match inline {
                Inline::Text(text) => push(out, Span { text: text.clone(), ..style.clone() }),
                Inline::Break => push(out, Span { text: "\n".into(), ..style.clone() }),
                Inline::Code(text) => push(out, Span { text: text.clone(), code: true, ..style.clone() }),
                Inline::Strong(inner) => walk(inner, &Span { strong: true, ..style.clone() }, out),
                Inline::Emphasis(inner) => walk(inner, &Span { emphasis: true, ..style.clone() }, out),
                Inline::Strike(inner) => walk(inner, &Span { strike: true, ..style.clone() }, out),
                Inline::Link { text, url } => walk(text, &Span { link: Some(url.clone()), ..style.clone() }, out),
            }
        }
    }
    fn push(out: &mut Vec<Span>, span: Span) {
        if span.text.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if same_style(last, &span) => last.text.push_str(&span.text),
            _ => out.push(span),
        }
    }
    fn same_style(a: &Span, b: &Span) -> bool {
        a.strong == b.strong && a.emphasis == b.emphasis && a.strike == b.strike && a.code == b.code && a.link == b.link
    }
    let mut out = Vec::new();
    walk(inlines, &Span::default(), &mut out);
    out
}

/// The plain text of inline content, for copying and for measuring.
pub fn plain(inlines: &[Inline]) -> String {
    spans(inlines).into_iter().map(|s| s.text).collect()
}

fn blocks(lines: &[&str]) -> Vec<Block> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        // Fenced code.
        if let Some((fence, lang)) = fence_open(trimmed) {
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !fence_close(lines[i], &fence) {
                body.push(strip_indent(lines[i], indent));
                i += 1;
            }
            let closed = i < lines.len();
            i += 1; // the closing fence, or the end of what has arrived
            out.push(code_or_widget(lang, body.join("\n"), closed));
            continue;
        }
        // Heading.
        if let Some((level, text)) = heading(trimmed) {
            out.push(Block::Heading { level, text: inline(text) });
            i += 1;
            continue;
        }
        // Rule.
        if is_rule(trimmed) {
            out.push(Block::Rule);
            i += 1;
            continue;
        }
        // Quote.
        if trimmed.starts_with('>') {
            let mut inner = Vec::new();
            while i < lines.len() {
                let t = lines[i].trim_start();
                let Some(rest) = t.strip_prefix('>') else { break };
                inner.push(rest.strip_prefix(' ').unwrap_or(rest));
                i += 1;
            }
            out.push(Block::Quote(blocks(&inner)));
            continue;
        }
        // Table: a row of cells followed by a row of dashes.
        if trimmed.contains('|') && i + 1 < lines.len() {
            if let Some(align) = table_rule(lines[i + 1]) {
                let head = cells(trimmed);
                if head.len() == align.len() {
                    i += 2;
                    let mut rows = Vec::new();
                    while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                        let mut row = cells(lines[i].trim());
                        row.resize(align.len(), Vec::new());
                        rows.push(row);
                        i += 1;
                    }
                    out.push(Block::Table { head, align, rows });
                    continue;
                }
            }
        }
        // List.
        if let Some(first) = marker(trimmed) {
            let (list, next) = list(lines, i, indent, first.ordered);
            out.push(list);
            i = next;
            continue;
        }
        // Paragraph: everything up to a blank line or something that starts a block.
        let mut text = vec![trimmed];
        i += 1;
        while i < lines.len() {
            let t = lines[i].trim_start();
            if t.is_empty() || fence_open(t).is_some() || heading(t).is_some() || is_rule(t) || t.starts_with('>') || marker(t).is_some() {
                break;
            }
            if t.contains('|') && i + 1 < lines.len() && table_rule(lines[i + 1]).is_some() {
                break;
            }
            text.push(t);
            i += 1;
        }
        out.push(Block::Paragraph(paragraph(&text)));
    }
    out
}

/// A paragraph's lines joined: a line ending in two spaces or a backslash breaks.
fn paragraph(lines: &[&str]) -> Vec<Inline> {
    let mut out = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let hard = line.ends_with("  ") || line.ends_with('\\');
        let text = line.trim_end().trim_end_matches('\\');
        out.extend(inline(text));
        if n + 1 < lines.len() {
            out.push(if hard { Inline::Break } else { Inline::Text(" ".into()) });
        }
    }
    merge_text(out)
}

fn fence_open(trimmed: &str) -> Option<(String, &str)> {
    let ch = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let count = trimmed.chars().take_while(|c| *c == ch).count();
    if count < 3 {
        return None;
    }
    let info = trimmed[count..].trim();
    // A backtick fence cannot have a backtick in its info string (that is inline code).
    if ch == '`' && info.contains('`') {
        return None;
    }
    Some((ch.to_string().repeat(count), info.split_whitespace().next().unwrap_or("")))
}

fn fence_close(line: &str, fence: &str) -> bool {
    let t = line.trim();
    let ch = fence.chars().next().unwrap_or('`');
    t.len() >= fence.len() && t.chars().all(|c| c == ch)
}

fn strip_indent(line: &str, indent: usize) -> &str {
    let leading = line.len() - line.trim_start().len();
    &line[leading.min(indent)..]
}

fn code_or_widget(lang: &str, text: String, closed: bool) -> Block {
    if lang == devd_genui::FENCE {
        return ui_block(lang, text, closed);
    }
    let kind = match lang {
        "chart" => Some(WidgetKind::Chart),
        "metrics" => Some(WidgetKind::Metrics),
        "steps" => Some(WidgetKind::Steps),
        "timeline" => Some(WidgetKind::Timeline),
        "details" => Some(WidgetKind::Details),
        _ => None,
    };
    if let Some(kind) = kind {
        if let Ok(data) = serde_json::from_str::<Value>(&text) {
            if data.is_object() {
                return Block::Widget { kind, data };
            }
        }
    }
    Block::Code { lang: lang.to_string(), text }
}

/// A `devd-ui` block: drawn from the catalog, as far as it has arrived and checks. A finished block that
/// is not JSON at all is shown as the code it is, so bad output is seen rather than lost.
fn ui_block(lang: &str, text: String, closed: bool) -> Block {
    if !closed {
        let tree = devd_genui::parse_partial(&text).and_then(|v| devd_genui::draft(&v));
        return Block::Ui { tree, state: UiState::Writing };
    }
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Block::Code { lang: lang.to_string(), text };
    };
    match devd_genui::check(&value) {
        Ok(tree) => Block::Ui { tree: Some(tree), state: UiState::Done },
        Err(issues) => Block::Ui { tree: devd_genui::draft(&value), state: UiState::Broken(issues) },
    }
}

fn heading(trimmed: &str) -> Option<(u8, &str)> {
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((level as u8, rest.trim().trim_end_matches('#').trim_end()))
}

fn is_rule(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ["-", "*", "_"].iter().any(|ch| compact.chars().all(|c| c.to_string() == *ch))
}

struct Marker {
    ordered: bool,
    start: u64,
    /// How many bytes the marker and its space take.
    width: usize,
}

fn marker(trimmed: &str) -> Option<Marker> {
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && matches!(bytes[0], b'-' | b'*' | b'+') && bytes[1] == b' ' {
        return Some(Marker { ordered: false, start: 1, width: 2 });
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let rest = &trimmed[digits..];
        if (rest.starts_with(". ") || rest.starts_with(") ")) && rest.len() > 2 || rest == "." {
            return Some(Marker { ordered: true, start: trimmed[..digits].parse().unwrap_or(1), width: digits + 2 });
        }
    }
    None
}

fn list(lines: &[&str], mut i: usize, indent: usize, ordered: bool) -> (Block, usize) {
    let mut items = Vec::new();
    let mut start = 1;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let this_indent = line.len() - trimmed.len();
        if trimmed.is_empty() {
            // A blank line ends the list unless the next line continues it.
            let next = lines[i + 1..].iter().find(|l| !l.trim().is_empty());
            let continues = next.is_some_and(|l| {
                let t = l.trim_start();
                let ind = l.len() - t.len();
                ind > indent || (ind == indent && marker(t).is_some_and(|m| m.ordered == ordered))
            });
            if !continues {
                break;
            }
            i += 1;
            continue;
        }
        if this_indent != indent {
            break;
        }
        let Some(m) = marker(trimmed).filter(|m| m.ordered == ordered) else { break };
        if items.is_empty() {
            start = m.start;
        }
        let content_indent = indent + m.width;
        let mut body: Vec<&str> = vec![trimmed.get(m.width..).unwrap_or("")];
        i += 1;
        // Lines indented past the marker belong to this entry.
        while i < lines.len() {
            let l = lines[i];
            let t = l.trim_start();
            let ind = l.len() - t.len();
            if t.is_empty() {
                let more = lines[i + 1..].iter().find(|x| !x.trim().is_empty()).is_some_and(|x| x.len() - x.trim_start().len() >= content_indent);
                if !more {
                    break;
                }
                body.push("");
                i += 1;
                continue;
            }
            if ind < content_indent && (ind <= indent || marker(t).is_some()) {
                break;
            }
            body.push(if ind >= content_indent { &l[content_indent..] } else { t });
            i += 1;
        }
        let (checked, first) = task(body[0]);
        body[0] = first;
        items.push(Item { checked, blocks: blocks(&body) });
    }
    (Block::List { ordered, start, items }, i)
}

fn task(text: &str) -> (Option<bool>, &str) {
    for (mark, checked) in [("[ ] ", false), ("[x] ", true), ("[X] ", true)] {
        if let Some(rest) = text.strip_prefix(mark) {
            return (Some(checked), rest);
        }
    }
    (None, text)
}

fn table_rule(line: &str) -> Option<Vec<Align>> {
    let t = line.trim();
    if !t.contains('-') || !t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
        return None;
    }
    let cols: Vec<Align> = split_row(t)
        .into_iter()
        .map(|cell| {
            let c = cell.trim();
            match (c.starts_with(':'), c.ends_with(':')) {
                (true, true) => Align::Center,
                (false, true) => Align::End,
                _ => Align::Start,
            }
        })
        .collect();
    (!cols.is_empty() && split_row(t).iter().all(|c| c.contains('-'))).then_some(cols)
}

fn split_row(row: &str) -> Vec<String> {
    let t = row.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut chars = t.chars().peekable();
    let mut in_code = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push('|');
                chars.next();
            }
            '`' => {
                in_code = !in_code;
                cell.push(c);
            }
            '|' if !in_code => out.push(std::mem::take(&mut cell)),
            _ => cell.push(c),
        }
    }
    out.push(cell);
    out
}

fn cells(row: &str) -> Vec<Vec<Inline>> {
    split_row(row).iter().map(|c| inline(c.trim())).collect()
}

/// Parse inline content.
pub fn inline(text: &str) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    merge_text(inline_range(&chars, 0, chars.len()))
}

fn merge_text(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        match (out.last_mut(), inline) {
            (Some(Inline::Text(last)), Inline::Text(next)) => last.push_str(&next),
            (_, other) => out.push(other),
        }
    }
    out
}

fn inline_range(c: &[char], start: usize, end: usize) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut i = start;
    let flush = |text: &mut String, out: &mut Vec<Inline>| {
        if !text.is_empty() {
            out.push(Inline::Text(std::mem::take(text)));
        }
    };
    while i < end {
        let ch = c[i];
        match ch {
            '\\' if i + 1 < end && c[i + 1].is_ascii_punctuation() => {
                text.push(c[i + 1]);
                i += 2;
            }
            '`' => {
                let ticks = run(c, i, end, '`');
                match find_run(c, i + ticks, end, '`', ticks) {
                    Some(close) => {
                        flush(&mut text, &mut out);
                        let body: String = c[i + ticks..close].iter().collect();
                        out.push(Inline::Code(body.trim_matches(' ').to_string()));
                        i = close + ticks;
                    }
                    None => {
                        text.extend(std::iter::repeat('`').take(ticks));
                        i += ticks;
                    }
                }
            }
            '*' | '_' | '~' => {
                let count = run(c, i, end, ch);
                let want = if ch == '~' { 2 } else { count.min(2) };
                // An underscore inside a word is part of the word (snake_case).
                let intraword = ch == '_' && i > start && c[i - 1].is_alphanumeric();
                let opens = count >= want && want > 0 && i + want < end && !c[i + want].is_whitespace() && !intraword && (ch != '~' || count >= 2);
                let close = if opens { find_closer(c, i + want, end, ch, want) } else { None };
                match close {
                    Some(close) => {
                        flush(&mut text, &mut out);
                        let inner = merge_text(inline_range(c, i + want, close));
                        out.push(match (ch, want) {
                            ('~', _) => Inline::Strike(inner),
                            (_, 2) => Inline::Strong(inner),
                            _ => Inline::Emphasis(inner),
                        });
                        i = close + want;
                    }
                    None => {
                        text.extend(std::iter::repeat(ch).take(count));
                        i += count;
                    }
                }
            }
            '[' => match link(c, i, end) {
                Some((label_end, url, after)) => {
                    flush(&mut text, &mut out);
                    let label = merge_text(inline_range(c, i + 1, label_end));
                    match safe_url(&url) {
                        Some(url) => out.push(Inline::Link { text: label, url }),
                        // An address the window will not open is shown as its text.
                        None => out.extend(label),
                    }
                    i = after;
                }
                None => {
                    text.push(ch);
                    i += 1;
                }
            },
            'h' if starts_with(c, i, end, "https://") || starts_with(c, i, end, "http://") => {
                let boundary = i == start || !c[i - 1].is_alphanumeric();
                let mut stop = i;
                while stop < end && !c[stop].is_whitespace() && !matches!(c[stop], '<' | '>' | '"' | '`') {
                    stop += 1;
                }
                // Punctuation that ends a sentence is not part of the address.
                while stop > i && matches!(c[stop - 1], '.' | ',' | ';' | ':' | '!' | '?' | ')' | '\'' | '*') {
                    stop -= 1;
                }
                let url: String = c[i..stop].iter().collect();
                if boundary && url.len() > 10 {
                    flush(&mut text, &mut out);
                    out.push(Inline::Link { text: vec![Inline::Text(url.clone())], url });
                    i = stop;
                } else {
                    text.push(ch);
                    i += 1;
                }
            }
            _ => {
                text.push(ch);
                i += 1;
            }
        }
    }
    flush(&mut text, &mut out);
    out
}

fn run(c: &[char], i: usize, end: usize, ch: char) -> usize {
    c[i..end].iter().take_while(|x| **x == ch).count()
}

/// The start of the next run of exactly `len` `ch`s.
fn find_run(c: &[char], mut i: usize, end: usize, ch: char, len: usize) -> Option<usize> {
    while i < end {
        if c[i] == ch {
            let n = run(c, i, end, ch);
            if n == len {
                return Some(i);
            }
            i += n;
        } else {
            i += 1;
        }
    }
    None
}

/// The closing delimiter for emphasis opened with `len` `ch`s.
fn find_closer(c: &[char], mut i: usize, end: usize, ch: char, len: usize) -> Option<usize> {
    while i < end {
        match c[i] {
            '\\' => i += 2,
            '`' => {
                // Skip a code span: delimiters inside it are literal.
                let ticks = run(c, i, end, '`');
                i = find_run(c, i + ticks, end, '`', ticks).map(|close| close + ticks).unwrap_or(i + ticks);
            }
            x if x == ch => {
                let n = run(c, i, end, ch);
                let after_word = ch != '_' || i + n >= end || !c[i + n].is_alphanumeric();
                if n >= len && !c[i - 1].is_whitespace() && after_word {
                    // `***a***`: the inner pair closes first, so take the last `len`.
                    return Some(i + n - len);
                }
                i += n;
            }
            _ => i += 1,
        }
    }
    None
}

/// `[label](url)` starting at `i`: the label's end, the url and where to continue.
fn link(c: &[char], i: usize, end: usize) -> Option<(usize, String, usize)> {
    let mut depth = 0;
    let mut j = i;
    let label_end = loop {
        if j >= end {
            return None;
        }
        match c[j] {
            '\\' => j += 1,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break j;
                }
            }
            _ => {}
        }
        j += 1;
    };
    if label_end + 1 >= end || c[label_end + 1] != '(' {
        return None;
    }
    let mut k = label_end + 2;
    let mut parens = 1;
    let mut url = String::new();
    while k < end {
        match c[k] {
            '(' => parens += 1,
            ')' => {
                parens -= 1;
                if parens == 0 {
                    // A title after the address (`(url "title")`) is dropped.
                    let url = url.split_whitespace().next().unwrap_or("").trim_matches(['<', '>']).to_string();
                    return Some((label_end, url, k + 1));
                }
            }
            _ => {}
        }
        url.push(c[k]);
        k += 1;
    }
    None
}

fn starts_with(c: &[char], i: usize, end: usize, prefix: &str) -> bool {
    let p: Vec<char> = prefix.chars().collect();
    i + p.len() <= end && c[i..i + p.len()] == p[..]
}

/// The address, when it is one the window is willing to open.
///
/// `javascript:`, `data:`, `file:` and anything else that is not a web or mail
/// address is refused: a reply cannot make a click do something else.
pub fn safe_url(url: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    let ok = lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:");
    ok.then(|| url.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(s: &str) -> Inline {
        Inline::Text(s.into())
    }

    #[test]
    fn headings_paragraphs_and_rules() {
        let doc = parse("# Title\n\nFirst line\nsecond line.\n\n---\n\n###### Six ##");
        assert_eq!(doc[0], Block::Heading { level: 1, text: vec![text("Title")] });
        assert_eq!(doc[1], Block::Paragraph(vec![text("First line second line.")]));
        assert_eq!(doc[2], Block::Rule);
        assert_eq!(doc[3], Block::Heading { level: 6, text: vec![text("Six")] });
        assert_eq!(parse("#hashtag")[0], Block::Paragraph(vec![text("#hashtag")]), "a heading needs a space");
    }

    #[test]
    fn emphasis_code_and_links() {
        let got = inline("a **bold *both*** `x*y` ~~gone~~ [docs](https://devd.test/a_b) done");
        assert_eq!(
            got,
            vec![
                text("a "),
                Inline::Strong(vec![text("bold "), Inline::Emphasis(vec![text("both")])]),
                text(" "),
                Inline::Code("x*y".into()),
                text(" "),
                Inline::Strike(vec![text("gone")]),
                text(" "),
                Inline::Link { text: vec![text("docs")], url: "https://devd.test/a_b".into() },
                text(" done"),
            ]
        );
    }

    #[test]
    fn identifiers_and_arithmetic_are_not_emphasis() {
        assert_eq!(inline("call snake_case_name and 2 * 3 * 4"), vec![text("call snake_case_name and 2 * 3 * 4")]);
        assert_eq!(inline("an unclosed **bold"), vec![text("an unclosed **bold")]);
        assert_eq!(inline(r"\*literal\*"), vec![text("*literal*")]);
    }

    #[test]
    fn a_bare_address_is_a_link_without_its_trailing_punctuation() {
        let got = inline("see https://example.com/docs.");
        assert_eq!(got[1], Inline::Link { text: vec![text("https://example.com/docs")], url: "https://example.com/docs".into() });
        assert_eq!(got[2], text("."));
    }

    #[test]
    fn an_address_that_is_not_web_or_mail_is_only_text() {
        assert_eq!(inline("[click](javascript:alert(1))"), vec![text("click")]);
        assert_eq!(inline("[x](data:text/html,hi)"), vec![text("x")]);
        assert_eq!(safe_url("  HTTPS://Example.com "), Some("HTTPS://Example.com".into()));
        assert_eq!(safe_url("file:///etc/passwd"), None);
    }

    #[test]
    fn html_is_text() {
        let doc = parse("<script>alert(1)</script>");
        assert_eq!(doc, vec![Block::Paragraph(vec![text("<script>alert(1)</script>")])]);
    }

    #[test]
    fn fenced_code_keeps_its_text_and_language() {
        let doc = parse("```rust\nfn main() {\n    let a = \"**not bold**\";\n}\n```\nafter");
        assert_eq!(doc[0], Block::Code { lang: "rust".into(), text: "fn main() {\n    let a = \"**not bold**\";\n}".into() });
        assert_eq!(doc[1], Block::Paragraph(vec![text("after")]));
    }

    #[test]
    fn a_reply_still_being_written_ends_inside_a_fence() {
        let doc = parse("Here:\n\n```sh\ncargo test");
        assert_eq!(doc[1], Block::Code { lang: "sh".into(), text: "cargo test".into() });
    }

    #[test]
    fn nested_and_numbered_lists() {
        let doc = parse("- one\n- two\n  - inner\n  - more\n- three\n\n3. third\n4. fourth");
        let Block::List { ordered: false, items, .. } = &doc[0] else { panic!("{doc:?}") };
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[1].blocks[1], Block::List { items, .. } if items.len() == 2));
        assert!(matches!(&doc[1], Block::List { ordered: true, start: 3, items } if items.len() == 2));
    }

    #[test]
    fn task_lists_carry_their_state() {
        let doc = parse("- [x] done\n- [ ] open\n- plain");
        let Block::List { items, .. } = &doc[0] else { panic!() };
        assert_eq!(items.iter().map(|i| i.checked).collect::<Vec<_>>(), [Some(true), Some(false), None]);
        assert_eq!(items[0].blocks[0], Block::Paragraph(vec![text("done")]));
    }

    #[test]
    fn a_list_entry_can_hold_a_code_block() {
        let doc = parse("1. Run it:\n\n   ```sh\n   devd doctor\n   ```\n2. Read it");
        let Block::List { items, .. } = &doc[0] else { panic!("{doc:?}") };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].blocks[1], Block::Code { lang: "sh".into(), text: "devd doctor".into() });
    }

    #[test]
    fn tables_with_alignment() {
        let doc = parse("| Name | Tests |\n|:-----|------:|\n| core | 412 |\n| `a\\|b` | 7 |");
        let Block::Table { head, align, rows } = &doc[0] else { panic!("{doc:?}") };
        assert_eq!(plain(&head[1]), "Tests");
        assert_eq!(align, &[Align::Start, Align::End]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][0], vec![Inline::Code("a|b".into())]);
    }

    #[test]
    fn a_pipe_in_prose_is_not_a_table() {
        let doc = parse("use a | b here\nand more");
        assert!(matches!(doc[0], Block::Paragraph(_)));
    }

    #[test]
    fn quotes_hold_blocks() {
        let doc = parse("> **Note**\n> second\n\nout");
        let Block::Quote(inner) = &doc[0] else { panic!() };
        assert_eq!(plain(match &inner[0] {
            Block::Paragraph(p) => p,
            _ => panic!(),
        }), "Note second");
        assert_eq!(doc.len(), 2);
    }

    #[test]
    fn a_widget_needs_valid_data_or_it_stays_code() {
        let good = parse("```metrics\n{\"items\":[{\"label\":\"Tests\",\"value\":12}]}\n```");
        assert_eq!(good[0], Block::Widget { kind: WidgetKind::Metrics, data: json!({"items":[{"label":"Tests","value":12}]}) });
        let bad = parse("```chart\n{not json\n```");
        assert!(matches!(&bad[0], Block::Code { lang, .. } if lang == "chart"));
    }

    #[test]
    fn spans_flatten_styles_and_merge_neighbours() {
        let got = spans(&inline("**a** **b** [`c`](https://x.test)"));
        assert_eq!(got.len(), 5, "{got:?}");
        assert!(got[0].strong && got[0].text == "a");
        assert!(!got[1].strong && got[1].text == " ");
        assert!(got[4].code && got[4].link.as_deref() == Some("https://x.test"));
        let merged = spans(&[Inline::Text("a".into()), Inline::Text("b".into())]);
        assert_eq!(merged.len(), 1, "neighbours with one style are one run");
    }

    #[test]
    fn hard_breaks() {
        let doc = parse("one  \ntwo\\\nthree");
        assert_eq!(doc[0], Block::Paragraph(vec![text("one"), Inline::Break, text("two"), Inline::Break, text("three")]));
    }

    #[test]
    fn a_devd_ui_block_is_drawn_as_far_as_it_has_arrived() {
        let whole = r#"{"type":"Card","props":{"title":"T"},"children":[{"type":"Text","props":{"text":"hello"}}]}"#;
        // Finished and valid.
        let done = parse(&format!("before

```devd-ui
{whole}
```

after"));
        assert!(matches!(&done[1], Block::Ui { tree: Some(t), state: UiState::Done } if t.children.len() == 1), "{done:?}");
        assert_eq!(done.len(), 3, "the text around it is still there");
        // Still being written: what has arrived is drawn.
        let writing = parse(&format!("```devd-ui
{}", &whole[..whole.len() - 20]));
        assert!(matches!(&writing[0], Block::Ui { tree: Some(_), state: UiState::Writing }), "{writing:?}");
        // Not even a root yet.
        assert!(matches!(&parse("```devd-ui
{\"ty")[0], Block::Ui { tree: None, state: UiState::Writing }));
    }

    #[test]
    fn a_wrong_devd_ui_block_keeps_what_can_be_drawn_and_says_what_is_wrong() {
        let wrong = r#"{"type":"Card","props":{"title":"T"},"children":[{"type":"Text","props":{"text":"ok"}},{"type":"Gauge"}]}"#;
        match &parse(&format!("```devd-ui
{wrong}
```"))[0] {
            Block::Ui { tree: Some(tree), state: UiState::Broken(issues) } => {
                assert_eq!(tree.children.len(), 1, "the good part is drawn");
                assert!(issues[0].message.contains("not a component"));
            }
            other => panic!("{other:?}"),
        }
        // Not JSON at all: shown as the code it is.
        assert!(matches!(&parse("```devd-ui
not json
```")[0], Block::Code { .. }));
    }
}
