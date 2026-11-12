//! Diffs: read from Git's text, or computed from two versions of a file.
//!
//! The viewer draws [`FileDiff`]s. There are two ways to get one. [`parse`]
//! reads the unified diff that `git.diff` returns. [`between`] compares two
//! texts itself, which is what shows an agent's edit before anything is
//! committed (`fs.edit` reports the old and the new text).
//!
//! Either way a changed line is paired with the line it replaced, and the pair
//! is compared word by word, so the viewer can mark the three characters that
//! changed and not the whole line.

/// The changes to one file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileDiff {
    /// Path before the change; empty for a new file.
    pub old_path: String,
    /// Path after the change; empty for a deleted file.
    pub new_path: String,
    /// What happened to the file as a whole.
    pub status: Status,
    /// Git says the content is binary; there are no hunks to show.
    pub binary: bool,
    /// The changed regions, in file order.
    pub hunks: Vec<Hunk>,
}

/// What happened to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    /// Its content changed.
    #[default]
    Modified,
    /// It is new.
    Added,
    /// It was removed.
    Deleted,
    /// It moved, with or without changes.
    Renamed,
}

/// One changed region with the unchanged lines around it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hunk {
    /// First line of the region in the old file (1-based).
    pub old_start: u32,
    /// First line of the region in the new file (1-based).
    pub new_start: u32,
    /// The text after the `@@` markers, usually the enclosing function.
    pub context: String,
    /// The lines, in order.
    pub lines: Vec<Line>,
}

/// One line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Unchanged, added or removed.
    pub kind: Kind,
    /// Its number in the old file, unless it was added.
    pub old_no: Option<u32>,
    /// Its number in the new file, unless it was removed.
    pub new_no: Option<u32>,
    /// The line without its `+`/`-` marker or its newline.
    pub text: String,
    /// Byte ranges of `text` that differ from the line it is paired with.
    /// Empty when the whole line is new, removed or unchanged.
    pub changed: Vec<(usize, usize)>,
}

/// Whether a line was kept, added or removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// In both versions.
    Context,
    /// Only in the new version.
    Added,
    /// Only in the old version.
    Removed,
}

impl FileDiff {
    /// The path to show: the new one, or the old one for a deleted file.
    pub fn path(&self) -> &str {
        if self.new_path.is_empty() { &self.old_path } else { &self.new_path }
    }

    /// Lines added and removed, the `+12 −3` beside a file's name.
    pub fn counts(&self) -> (usize, usize) {
        let lines = self.hunks.iter().flat_map(|h| &h.lines);
        lines.fold((0, 0), |(a, r), l| match l.kind {
            Kind::Added => (a + 1, r),
            Kind::Removed => (a, r + 1),
            Kind::Context => (a, r),
        })
    }
}

/// Read a unified diff, as `git diff` writes it, into one [`FileDiff`] per file.
pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut old_no = 0;
    let mut new_no = 0;
    for raw in text.lines() {
        if let Some(rest) = raw.strip_prefix("diff --git ") {
            let (a, b) = split_git_paths(rest);
            files.push(FileDiff { old_path: a, new_path: b, ..FileDiff::default() });
            continue;
        }
        // A diff with no `diff --git` header (plain `diff -u`) starts at `---`.
        if raw.starts_with("--- ") && files.last().map_or(true, |f| !f.hunks.is_empty()) {
            files.push(FileDiff::default());
        }
        let Some(file) = files.last_mut() else { continue };
        if file.hunks.is_empty() {
            if let Some(path) = raw.strip_prefix("--- ") {
                file.old_path = header_path(path);
                if file.old_path.is_empty() {
                    file.status = Status::Added;
                }
                continue;
            }
            if let Some(path) = raw.strip_prefix("+++ ") {
                file.new_path = header_path(path);
                if file.new_path.is_empty() {
                    file.status = Status::Deleted;
                }
                continue;
            }
            if raw.starts_with("new file mode") {
                file.status = Status::Added;
                continue;
            }
            if raw.starts_with("deleted file mode") {
                file.status = Status::Deleted;
                continue;
            }
            if let Some(path) = raw.strip_prefix("rename from ") {
                file.status = Status::Renamed;
                file.old_path = path.to_string();
                continue;
            }
            if let Some(path) = raw.strip_prefix("rename to ") {
                file.status = Status::Renamed;
                file.new_path = path.to_string();
                continue;
            }
            if raw.starts_with("Binary files ") || raw.starts_with("GIT binary patch") {
                file.binary = true;
                continue;
            }
        }
        if let Some((o, n, context)) = hunk_header(raw) {
            old_no = o;
            new_no = n;
            file.hunks.push(Hunk { old_start: o, new_start: n, context, lines: Vec::new() });
            continue;
        }
        let Some(hunk) = file.hunks.last_mut() else { continue };
        let (kind, body) = match raw.as_bytes().first() {
            Some(b'+') => (Kind::Added, &raw[1..]),
            Some(b'-') => (Kind::Removed, &raw[1..]),
            Some(b' ') => (Kind::Context, &raw[1..]),
            // "\ No newline at end of file" describes the line above; it is not a line.
            Some(b'\\') => continue,
            // An empty context line that lost its leading space in transit.
            None => (Kind::Context, ""),
            _ => continue,
        };
        let line = match kind {
            Kind::Added => {
                new_no += 1;
                Line { kind, old_no: None, new_no: Some(new_no - 1), text: body.to_string(), changed: Vec::new() }
            }
            Kind::Removed => {
                old_no += 1;
                Line { kind, old_no: Some(old_no - 1), new_no: None, text: body.to_string(), changed: Vec::new() }
            }
            Kind::Context => {
                old_no += 1;
                new_no += 1;
                Line { kind, old_no: Some(old_no - 1), new_no: Some(new_no - 1), text: body.to_string(), changed: Vec::new() }
            }
        };
        hunk.lines.push(line);
    }
    for file in &mut files {
        if file.status == Status::Deleted {
            file.new_path.clear();
        }
        if file.status == Status::Added {
            file.old_path.clear();
        }
        for hunk in &mut file.hunks {
            mark_changes(&mut hunk.lines);
        }
    }
    files
}

/// `a/src/x.rs b/src/x.rs` as two paths. Paths with spaces are quoted by Git.
fn split_git_paths(rest: &str) -> (String, String) {
    let unquote = |s: &str| s.trim_matches('"').to_string();
    let strip = |s: String, prefix: &str| s.strip_prefix(prefix).map(str::to_string).unwrap_or(s);
    if let Some(mid) = rest.find(" b/") {
        return (strip(unquote(&rest[..mid]), "a/"), strip(unquote(&rest[mid + 1..]), "b/"));
    }
    if let Some(mid) = rest.find("\" \"") {
        return (strip(unquote(&rest[..=mid]), "a/"), strip(unquote(&rest[mid + 2..]), "b/"));
    }
    (String::new(), String::new())
}

fn header_path(path: &str) -> String {
    // A timestamp may follow a tab in plain `diff -u` output.
    let path = path.split('\t').next().unwrap_or(path).trim().trim_matches('"');
    if path == "/dev/null" {
        return String::new();
    }
    path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")).unwrap_or(path).to_string()
}

/// `@@ -12,7 +12,9 @@ fn name()` as (old start, new start, context).
fn hunk_header(line: &str) -> Option<(u32, u32, String)> {
    let rest = line.strip_prefix("@@ -")?;
    let (ranges, context) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let start = |r: &str| r.split(',').next().and_then(|n| n.parse::<u32>().ok());
    Some((start(old)?, start(new)?, context.trim().to_string()))
}

/// Pair each run of removed lines with the added lines that follow it, and mark
/// what differs inside each pair.
fn mark_changes(lines: &mut [Line]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != Kind::Removed {
            i += 1;
            continue;
        }
        let removed_end = (i..lines.len()).find(|j| lines[*j].kind != Kind::Removed).unwrap_or(lines.len());
        let added_end = (removed_end..lines.len()).find(|j| lines[*j].kind != Kind::Added).unwrap_or(lines.len());
        let removed = removed_end - i;
        let added = added_end - removed_end;
        // Only a like-for-like replacement is paired. Three lines replaced by
        // seven is a rewrite, and marking words in it would be noise.
        if removed == added {
            for k in 0..removed {
                let (old, new) = (lines[i + k].text.clone(), lines[removed_end + k].text.clone());
                if let Some((old_ranges, new_ranges)) = inline_change(&old, &new) {
                    lines[i + k].changed = old_ranges;
                    lines[removed_end + k].changed = new_ranges;
                }
            }
        }
        i = added_end.max(i + 1);
    }
}

/// Byte ranges into a line, in order.
pub type Ranges = Vec<(usize, usize)>;

/// The byte ranges that differ between two versions of a line, word by word.
///
/// `None` when the lines have too little in common for the marks to mean
/// anything; then the whole line is the change.
pub fn inline_change(old: &str, new: &str) -> Option<(Ranges, Ranges)> {
    let a = words(old);
    let b = words(new);
    let ops = lcs(&a, &b, |x, y| old[x.0..x.1] == new[y.0..y.1]);
    let kept: usize = ops.iter().filter(|op| matches!(op, Op::Keep(..))).map(|op| if let Op::Keep(i, _) = op { a[*i].1 - a[*i].0 } else { 0 }).sum();
    let longest = old.len().max(new.len());
    if longest == 0 || kept * 3 < longest {
        return None;
    }
    let mut old_ranges: Ranges = Vec::new();
    let mut new_ranges: Ranges = Vec::new();
    let push = |ranges: &mut Ranges, range: (usize, usize)| match ranges.last_mut() {
        Some(last) if last.1 == range.0 => last.1 = range.1,
        _ => ranges.push(range),
    };
    for op in ops {
        match op {
            Op::Remove(i) => push(&mut old_ranges, a[i]),
            Op::Add(j) => push(&mut new_ranges, b[j]),
            Op::Keep(..) => {}
        }
    }
    Some((old_ranges, new_ranges))
}

/// A line cut into words, runs of spaces and single punctuation marks.
fn words(line: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut class = None;
    for (i, ch) in line.char_indices() {
        let this = if ch.is_alphanumeric() || ch == '_' {
            1
        } else if ch.is_whitespace() {
            2
        } else {
            3
        };
        let split = match class {
            None => false,
            Some(previous) => previous != this || this == 3,
        };
        if split {
            out.push((start, i));
            start = i;
        }
        class = Some(this);
    }
    if start < line.len() {
        out.push((start, line.len()));
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Keep(usize, usize),
    Remove(usize),
    Add(usize),
}

/// The edit script between two sequences, by longest common subsequence.
///
/// The common prefix and suffix are set aside first, which is nearly all of a
/// typical edit, so the quadratic table only covers the part that changed.
fn lcs<T>(a: &[T], b: &[T], same: impl Fn(&T, &T) -> bool) -> Vec<Op> {
    let mut prefix = 0;
    while prefix < a.len() && prefix < b.len() && same(&a[prefix], &b[prefix]) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < a.len() - prefix && suffix < b.len() - prefix && same(&a[a.len() - 1 - suffix], &b[b.len() - 1 - suffix]) {
        suffix += 1;
    }
    let (am, bm) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let mut ops: Vec<Op> = (0..prefix).map(|i| Op::Keep(i, i)).collect();
    // A middle too large to tabulate is reported as replaced outright.
    if am.len().saturating_mul(bm.len()) > 4_000_000 {
        ops.extend((0..am.len()).map(|i| Op::Remove(prefix + i)));
        ops.extend((0..bm.len()).map(|j| Op::Add(prefix + j)));
    } else {
        let (n, m) = (am.len(), bm.len());
        let mut table = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                table[i * (m + 1) + j] = if same(&am[i], &bm[j]) {
                    table[(i + 1) * (m + 1) + j + 1] + 1
                } else {
                    table[(i + 1) * (m + 1) + j].max(table[i * (m + 1) + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if same(&am[i], &bm[j]) {
                ops.push(Op::Keep(prefix + i, prefix + j));
                i += 1;
                j += 1;
            } else if table[(i + 1) * (m + 1) + j] >= table[i * (m + 1) + j + 1] {
                ops.push(Op::Remove(prefix + i));
                i += 1;
            } else {
                ops.push(Op::Add(prefix + j));
                j += 1;
            }
        }
        ops.extend((i..n).map(|i| Op::Remove(prefix + i)));
        ops.extend((j..m).map(|j| Op::Add(prefix + j)));
    }
    ops.extend((0..suffix).map(|k| Op::Keep(a.len() - suffix + k, b.len() - suffix + k)));
    ops
}

/// Compare two versions of a file, keeping `context` unchanged lines around each change.
pub fn between(path: &str, old: &str, new: &str, context: usize) -> FileDiff {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let ops = lcs(&a, &b, |x, y| x == y);
    let changed: Vec<usize> = ops.iter().enumerate().filter(|(_, op)| !matches!(op, Op::Keep(..))).map(|(i, _)| i).collect();
    let mut file = FileDiff {
        old_path: if old.is_empty() { String::new() } else { path.to_string() },
        new_path: if new.is_empty() && !old.is_empty() { String::new() } else { path.to_string() },
        status: match (old.is_empty(), new.is_empty()) {
            (true, false) => Status::Added,
            (false, true) => Status::Deleted,
            _ => Status::Modified,
        },
        ..FileDiff::default()
    };
    // Group changes whose context would touch into one hunk.
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for at in changed {
        match groups.last_mut() {
            Some(last) if at <= last.1 + 2 * context + 1 => last.1 = at,
            _ => groups.push((at, at)),
        }
    }
    for (first, last) in groups {
        let from = first.saturating_sub(context);
        let to = (last + context + 1).min(ops.len());
        let mut old_no = 1 + ops[..from].iter().filter(|op| !matches!(op, Op::Add(_))).count() as u32;
        let mut new_no = 1 + ops[..from].iter().filter(|op| !matches!(op, Op::Remove(_))).count() as u32;
        let mut hunk = Hunk { old_start: old_no, new_start: new_no, context: String::new(), lines: Vec::new() };
        for op in &ops[from..to] {
            hunk.lines.push(match *op {
                Op::Keep(i, _) => {
                    let line = Line { kind: Kind::Context, old_no: Some(old_no), new_no: Some(new_no), text: a[i].to_string(), changed: Vec::new() };
                    old_no += 1;
                    new_no += 1;
                    line
                }
                Op::Remove(i) => {
                    let line = Line { kind: Kind::Removed, old_no: Some(old_no), new_no: None, text: a[i].to_string(), changed: Vec::new() };
                    old_no += 1;
                    line
                }
                Op::Add(j) => {
                    let line = Line { kind: Kind::Added, old_no: None, new_no: Some(new_no), text: b[j].to_string(), changed: Vec::new() };
                    new_no += 1;
                    line
                }
            });
        }
        mark_changes(&mut hunk.lines);
        file.hunks.push(hunk);
    }
    file
}

/// A row of the side-by-side view: what is on the left and what is on the right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row<'a> {
    /// The old line, or nothing where the right side was added.
    pub left: Option<&'a Line>,
    /// The new line, or nothing where the left side was removed.
    pub right: Option<&'a Line>,
}

/// Lay a hunk out in two columns, a removed line beside the line that replaced it.
pub fn side_by_side(hunk: &Hunk) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    let lines = &hunk.lines;
    let mut i = 0;
    while i < lines.len() {
        match lines[i].kind {
            Kind::Context => {
                rows.push(Row { left: Some(&lines[i]), right: Some(&lines[i]) });
                i += 1;
            }
            Kind::Added => {
                rows.push(Row { left: None, right: Some(&lines[i]) });
                i += 1;
            }
            Kind::Removed => {
                let removed_end = (i..lines.len()).find(|j| lines[*j].kind != Kind::Removed).unwrap_or(lines.len());
                let added_end = (removed_end..lines.len()).find(|j| lines[*j].kind != Kind::Added).unwrap_or(lines.len());
                let (removed, added) = (&lines[i..removed_end], &lines[removed_end..added_end]);
                for k in 0..removed.len().max(added.len()) {
                    rows.push(Row { left: removed.get(k), right: added.get(k) });
                }
                i = added_end;
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIT: &str = "diff --git a/src/lib.rs b/src/lib.rs
index 1a2b3c4..5d6e7f8 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,6 +10,7 @@ fn main() {
     let a = 1;
-    let total = count + 1;
+    let total = count + 2;
+    log(total);
     done();
\\ No newline at end of file
diff --git a/old name.txt b/new name.txt
similarity index 100%
rename from old name.txt
rename to new name.txt
diff --git a/logo.png b/logo.png
new file mode 100644
Binary files /dev/null and b/logo.png differ
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
--- a/gone.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-fn gone() {}
-
";

    #[test]
    fn a_git_diff_becomes_files_hunks_and_numbered_lines() {
        let files = parse(GIT);
        assert_eq!(files.len(), 4);
        let lib = &files[0];
        assert_eq!((lib.path(), lib.status), ("src/lib.rs", Status::Modified));
        assert_eq!(lib.counts(), (2, 1));
        let hunk = &lib.hunks[0];
        assert_eq!((hunk.old_start, hunk.new_start, hunk.context.as_str()), (10, 10, "fn main() {"));
        let numbers: Vec<_> = hunk.lines.iter().map(|l| (l.old_no, l.new_no)).collect();
        assert_eq!(numbers, [(Some(10), Some(10)), (Some(11), None), (None, Some(11)), (None, Some(12)), (Some(12), Some(13))]);
    }

    #[test]
    fn renames_binaries_and_deletions_are_recognised() {
        let files = parse(GIT);
        assert_eq!((files[1].status, files[1].old_path.as_str(), files[1].path()), (Status::Renamed, "old name.txt", "new name.txt"));
        assert!(files[2].binary && files[2].status == Status::Added && files[2].hunks.is_empty());
        assert_eq!((files[3].status, files[3].path(), files[3].counts()), (Status::Deleted, "gone.rs", (0, 2)));
    }

    #[test]
    fn a_replaced_line_marks_only_what_changed_in_it() {
        let (old, new) = inline_change("    let total = count + 1;", "    let total = count + 2;").unwrap();
        assert_eq!(&"    let total = count + 1;"[old[0].0..old[0].1], "1");
        assert_eq!(&"    let total = count + 2;"[new[0].0..new[0].1], "2");
        assert_eq!((old.len(), new.len()), (1, 1));
    }

    #[test]
    fn unrelated_lines_are_not_marked_word_by_word() {
        assert_eq!(inline_change("fn alpha(x: u8) -> bool {", "struct Totally { different: String }"), None);
    }

    #[test]
    fn unequal_replacements_are_left_unpaired() {
        // One line replaced by two: the first hunk above. Neither side is marked.
        let files = parse(GIT);
        assert!(files[0].hunks[0].lines.iter().all(|l| l.changed.is_empty()));
        let paired = parse("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-let a = 1;\n+let a = 2;\n");
        assert_eq!(paired[0].hunks[0].lines[0].changed, [(8, 9)]);
    }

    #[test]
    fn two_texts_are_compared_with_context_around_each_change() {
        let old = (1..=20).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
        let new = old.replace("line 5", "line five").replace("line 16\n", "");
        let file = between("notes.txt", &old, &new, 2);
        assert_eq!(file.status, Status::Modified);
        assert_eq!(file.hunks.len(), 2, "changes eleven lines apart are separate hunks");
        assert_eq!((file.hunks[0].old_start, file.hunks[0].new_start), (3, 3));
        assert_eq!(file.hunks[0].lines.len(), 6);
        assert_eq!((file.hunks[1].old_start, file.hunks[1].new_start), (14, 14));
        assert_eq!(file.counts(), (1, 2));
    }

    #[test]
    fn a_new_file_and_an_emptied_file() {
        let added = between("a.txt", "", "one\ntwo", 3);
        assert_eq!((added.status, added.counts()), (Status::Added, (2, 0)));
        let removed = between("a.txt", "one\ntwo", "", 3);
        assert_eq!((removed.status, removed.path(), removed.counts()), (Status::Deleted, "a.txt", (0, 2)));
        assert!(between("a.txt", "same", "same", 3).hunks.is_empty());
    }

    #[test]
    fn side_by_side_puts_a_replacement_on_one_row() {
        let files = parse(GIT);
        let rows = side_by_side(&files[0].hunks[0]);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].left.unwrap().text, "    let total = count + 1;");
        assert_eq!(rows[1].right.unwrap().text, "    let total = count + 2;");
        assert!(rows[2].left.is_none() && rows[2].right.unwrap().text == "    log(total);");
    }
}
