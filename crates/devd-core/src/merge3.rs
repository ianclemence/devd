//! A three-way text merge.
//!
//! Given the text two parties started from (`base`) and what each made of it
//! (`ours`, `theirs`), produce the text that has both sets of changes, or say
//! where they collide. Changes to different lines combine; changes to the same
//! lines combine only when they are identical. Nothing is guessed.
//!
//! This is the merge that lets two proposals made against the same file both be
//! applied, and it is also what a conflict-resolving agent's result is checked
//! against: a resolution that is not one of the sides must be justified by
//! evidence, not by being plausible.

use crate::{DevdError, Result};

/// Largest comparison (lines x lines) the merge will attempt.
const MAX_CELLS: usize = 4_000_000;

/// How to treat two different pure insertions at the same point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inserts {
    /// A collision, as in any three-way merge.
    Conflict,
    /// Keep both, ours first, without repeating a line ours already has and without
    /// the other side's blank lines. Right for independent list items, and only for
    /// them.
    Union,
}

/// A region both sides changed differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// First base line of the region (0-based).
    pub start: usize,
    /// One past the last base line of the region.
    pub end: usize,
    /// What our side made of it.
    pub ours: Vec<String>,
    /// What their side made of it.
    pub theirs: Vec<String>,
}

/// The result of a merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merged {
    /// Both sets of changes, combined.
    Clean(String),
    /// Regions that need a decision.
    Conflicts(Vec<Conflict>),
}

#[derive(Debug, Clone)]
struct Edit {
    start: usize,
    end: usize,
    repl: Vec<String>,
}

fn lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(str::to_string).collect()
}

/// The changes that turn `base` into `side`, in base coordinates.
fn edits(base: &[String], side: &[String]) -> Result<Vec<Edit>> {
    let (n, m) = (base.len(), side.len());
    if n.saturating_mul(m) > MAX_CELLS {
        return Err(DevdError::invalid(
            "the files are too large to merge line by line; resolve by hand",
        ));
    }
    // lcs[i][j] = length of the longest common subsequence of base[i..] and side[j..].
    let width = m + 1;
    let mut lcs = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i * width + j] = if base[i] == side[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let (mut gap_i, mut gap_j) = (0usize, 0usize);
    while i < n || j < m {
        if i < n && j < m && base[i] == side[j] {
            if gap_i != i || gap_j != j {
                out.push(Edit {
                    start: gap_i,
                    end: i,
                    repl: side[gap_j..j].to_vec(),
                });
            }
            i += 1;
            j += 1;
            gap_i = i;
            gap_j = j;
        } else if j < m && (i == n || lcs[i * width + j + 1] >= lcs[(i + 1) * width + j]) {
            j += 1;
        } else {
            i += 1;
        }
    }
    if gap_i != n || gap_j != m {
        out.push(Edit {
            start: gap_i,
            end: n,
            repl: side[gap_j..m].to_vec(),
        });
    }
    Ok(out)
}

/// Base lines `start..end` with `edits` applied (all of them inside the range).
fn apply_range(base: &[String], start: usize, end: usize, edits: &[Edit]) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = start;
    for e in edits {
        out.extend_from_slice(&base[pos..e.start]);
        out.extend(e.repl.iter().cloned());
        pos = e.end;
    }
    out.extend_from_slice(&base[pos..end]);
    out
}

/// Merge `ours` and `theirs`, which both descend from `base`.
pub fn merge(base: &str, ours: &str, theirs: &str, inserts: Inserts) -> Result<Merged> {
    if ours == theirs {
        return Ok(Merged::Clean(ours.to_string()));
    }
    if ours == base {
        return Ok(Merged::Clean(theirs.to_string()));
    }
    if theirs == base {
        return Ok(Merged::Clean(ours.to_string()));
    }
    let b = lines(base);
    let o_edits = edits(&b, &lines(ours))?;
    let t_edits = edits(&b, &lines(theirs))?;

    let mut out: Vec<String> = Vec::new();
    let mut conflicts = Vec::new();
    let (mut oi, mut ti) = (0usize, 0usize);
    let mut pos = 0usize;

    while oi < o_edits.len() || ti < t_edits.len() {
        // The group starts at the earlier of the two next edits and grows while
        // an edit from the other side touches it. Touching means overlapping, or
        // meeting at a point where either side inserts.
        let next_o = o_edits.get(oi);
        let next_t = t_edits.get(ti);
        let start = match (next_o, next_t) {
            (Some(o), Some(t)) => o.start.min(t.start),
            (Some(o), None) => o.start,
            (None, Some(t)) => t.start,
            (None, None) => break,
        };
        let (mut end, mut o_hi, mut t_hi) = (start, oi, ti);
        loop {
            let mut grew = false;
            while let Some(e) = o_edits.get(o_hi) {
                if e.start < end || (e.start == end && (o_hi == oi || e.start == e.end || end == start)) {
                    end = end.max(e.end);
                    o_hi += 1;
                    grew = true;
                } else {
                    break;
                }
            }
            while let Some(e) = t_edits.get(t_hi) {
                if e.start < end || (e.start == end && (t_hi == ti || e.start == e.end || end == start)) {
                    end = end.max(e.end);
                    t_hi += 1;
                    grew = true;
                } else {
                    break;
                }
            }
            if !grew {
                break;
            }
        }
        out.extend_from_slice(&b[pos..start]);
        let o_group = &o_edits[oi..o_hi];
        let t_group = &t_edits[ti..t_hi];
        if t_group.is_empty() {
            out.extend(apply_range(&b, start, end, o_group));
        } else if o_group.is_empty() {
            out.extend(apply_range(&b, start, end, t_group));
        } else {
            let ours_text = apply_range(&b, start, end, o_group);
            let theirs_text = apply_range(&b, start, end, t_group);
            let only_inserts = o_group.iter().chain(t_group).all(|e| e.start == e.end);
            if ours_text == theirs_text {
                out.extend(ours_text);
            } else if inserts == Inserts::Union && only_inserts {
                out.extend(ours_text.iter().cloned());
                // An insertion at the end of a file that has no final newline
                // would otherwise run into the next one.
                if out.last().is_some_and(|l| !l.ends_with('\n')) {
                    if let Some(last) = out.last_mut() {
                        last.push('\n');
                    }
                }
                // Two sides that each add the same heading and then their own item
                // must not produce two headings: lines ours already has, and blank
                // separators, are not repeated.
                out.extend(
                    theirs_text
                        .into_iter()
                        .filter(|l| !l.trim().is_empty() && !ours_text.contains(l)),
                );
            } else {
                conflicts.push(Conflict {
                    start,
                    end,
                    ours: ours_text,
                    theirs: theirs_text,
                });
            }
        }
        pos = end;
        oi = o_hi;
        ti = t_hi;
    }
    if !conflicts.is_empty() {
        return Ok(Merged::Conflicts(conflicts));
    }
    out.extend_from_slice(&b[pos..]);
    Ok(Merged::Clean(out.concat()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(m: Merged) -> String {
        match m {
            Merged::Clean(t) => t,
            Merged::Conflicts(c) => panic!("unexpected conflicts: {c:?}"),
        }
    }

    #[test]
    fn changes_to_different_lines_combine() {
        let base = "a\nb\nc\nd\ne\n";
        let ours = "A\nb\nc\nd\ne\n";
        let theirs = "a\nb\nc\nd\nE\n";
        assert_eq!(
            clean(merge(base, ours, theirs, Inserts::Conflict).unwrap()),
            "A\nb\nc\nd\nE\n"
        );
    }

    #[test]
    fn the_same_change_on_both_sides_is_made_once() {
        let base = "a\nb\nc\n";
        let both = "a\nB\nc\n";
        assert_eq!(clean(merge(base, both, both, Inserts::Conflict).unwrap()), both);
        // Identical edits that arrive by different routes also collapse.
        let ours = "a\nB\nc\nx\n";
        let theirs = "a\nB\nc\n";
        assert_eq!(
            clean(merge(base, ours, theirs, Inserts::Conflict).unwrap()),
            "a\nB\nc\nx\n"
        );
    }

    #[test]
    fn different_changes_to_the_same_line_are_a_conflict_with_both_versions() {
        let base = "a\nb\nc\n";
        let ours = "a\nours\nc\n";
        let theirs = "a\ntheirs\nc\n";
        match merge(base, ours, theirs, Inserts::Conflict).unwrap() {
            Merged::Conflicts(c) => {
                assert_eq!(c.len(), 1);
                assert_eq!((c[0].start, c[0].end), (1, 2));
                assert_eq!(c[0].ours, vec!["ours\n"]);
                assert_eq!(c[0].theirs, vec!["theirs\n"]);
            }
            Merged::Clean(t) => panic!("merged into {t:?}"),
        }
    }

    #[test]
    fn two_insertions_at_one_point_collide_unless_the_caller_says_to_keep_both() {
        let base = "# Memory\n- one\n";
        let ours = "# Memory\n- one\n- two\n";
        let theirs = "# Memory\n- one\n- three\n";
        assert!(matches!(
            merge(base, ours, theirs, Inserts::Conflict).unwrap(),
            Merged::Conflicts(_)
        ));
        assert_eq!(
            clean(merge(base, ours, theirs, Inserts::Union).unwrap()),
            "# Memory\n- one\n- two\n- three\n"
        );
    }

    #[test]
    fn an_edit_and_a_deletion_of_the_same_line_conflict_but_distant_ones_do_not() {
        let base = "a\nb\nc\nd\n";
        let edit_b = "a\nB\nc\nd\n";
        let delete_b = "a\nc\nd\n";
        assert!(matches!(
            merge(base, edit_b, delete_b, Inserts::Conflict).unwrap(),
            Merged::Conflicts(_)
        ));
        let delete_d = "a\nb\nc\n";
        assert_eq!(
            clean(merge(base, edit_b, delete_d, Inserts::Conflict).unwrap()),
            "a\nB\nc\n"
        );
    }

    #[test]
    fn an_untouched_side_returns_the_other_unchanged() {
        assert_eq!(clean(merge("x\n", "x\n", "y\n", Inserts::Conflict).unwrap()), "y\n");
        assert_eq!(clean(merge("x\n", "y\n", "x\n", Inserts::Conflict).unwrap()), "y\n");
        assert_eq!(clean(merge("", "a\n", "b\n", Inserts::Union).unwrap()), "a\nb\n");
    }

    #[test]
    fn appending_to_a_last_line_with_no_newline_changes_that_line_and_so_conflicts() {
        // The final line is rewritten by both sides (it gains a newline), which
        // is a change to the same line, not two independent insertions.
        let merged = merge("- one", "- one
- two", "- one
- three", Inserts::Union).unwrap();
        assert!(matches!(merged, Merged::Conflicts(_)));
    }

    #[test]
    fn an_enormous_comparison_is_refused_rather_than_attempted() {
        let big: String = (0..3000).map(|i| format!("line {i}\n")).collect();
        let other: String = (0..3000).map(|i| format!("other {i}\n")).collect();
        assert!(merge(&big, &other, &format!("{big}x\n"), Inserts::Conflict).is_err());
    }
}
