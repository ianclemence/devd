//! The audit log as a chain.
//!
//! Every row carries the hash of the row before it, and its own hash covers its
//! content and that link. Change a row, remove one from the middle or put them in
//! another order, and every hash after that point stops matching: the record can no
//! longer say it is intact. That is *tamper-evident*, not tamper-proof: someone who
//! can rewrite the whole table can rewrite the chain too. So two things narrow what
//! they can get away with:
//!
//! * **The head is kept apart.** The newest row's id and hash are also saved in
//!   settings. Cutting the end off the log (which no chain can see on its own) leaves
//!   the saved head pointing at a row that is gone.
//! * **Pruning leaves an anchor.** Old rows are deleted to keep the log small. The
//!   last row removed is remembered, and the first row kept must link to it, so a
//!   pruned log verifies and a pruned-and-edited one does not.
//!
//! Rows written before chaining existed have no hash. They are counted and named, not
//! treated as broken: the chain starts at the first row that has one.
//!
//! This module is only the arithmetic, so each way of tampering has a test that runs
//! in microseconds.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The part of a row the hash covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fields<'a> {
    /// When it was written.
    pub at: &'a str,
    /// Who did it.
    pub actor: &'a str,
    /// How it arrived.
    pub surface: &'a str,
    /// What was asked for.
    pub capability: &'a str,
    /// Its tier.
    pub tier: &'a str,
    /// What it does.
    pub effect: &'a str,
    /// Whether it was only a plan.
    pub dry_run: bool,
    /// Whether it worked.
    pub ok: bool,
    /// The recorded arguments.
    pub args: &'a str,
    /// What came of it.
    pub message: &'a str,
}

/// The hash of a row given the one before it (empty for the first).
///
/// Each field is length-prefixed, so no two different rows can produce the same bytes
/// by moving text across a field boundary.
pub fn link(prev: &str, f: &Fields) -> String {
    let mut h = Sha256::new();
    let mut put = |s: &str| {
        h.update((s.len() as u64).to_le_bytes());
        h.update(s.as_bytes());
    };
    put("devd.audit.v1");
    put(prev);
    for s in [f.at, f.actor, f.surface, f.capability, f.tier, f.effect, f.args, f.message] {
        put(s);
    }
    put(if f.dry_run { "1" } else { "0" });
    put(if f.ok { "1" } else { "0" });
    hex::encode(h.finalize())
}

/// One stored row, as the check reads it.
#[derive(Debug, Clone)]
pub struct Row {
    /// Row id.
    pub id: i64,
    /// The hash of the row before it, as written.
    pub prev_hash: String,
    /// Its own hash, as written. Empty for a row from before chaining.
    pub hash: String,
    /// What the hash covers.
    pub at: String,
    /// Actor.
    pub actor: String,
    /// Surface.
    pub surface: String,
    /// Capability.
    pub capability: String,
    /// Tier.
    pub tier: String,
    /// Effect.
    pub effect: String,
    /// Plan only.
    pub dry_run: bool,
    /// Worked.
    pub ok: bool,
    /// Arguments.
    pub args: String,
    /// Message.
    pub message: String,
}

impl Row {
    fn fields(&self) -> Fields<'_> {
        Fields {
            at: &self.at,
            actor: &self.actor,
            surface: &self.surface,
            capability: &self.capability,
            tier: &self.tier,
            effect: &self.effect,
            dry_run: self.dry_run,
            ok: self.ok,
            args: &self.args,
            message: &self.message,
        }
    }
}

/// Where the chain stops being true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Break {
    /// The row where it was first noticed.
    pub id: i64,
    /// What is wrong, in words.
    pub why: String,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Verification {
    /// Whether the chain is intact: no break, and the saved head still matches.
    pub intact: bool,
    /// Rows checked.
    pub checked: usize,
    /// Rows from before chaining, which cannot be checked.
    pub unchained: usize,
    /// The first break, if any.
    pub first_break: Option<Break>,
    /// The newest row's hash, to pin somewhere else.
    pub head: Option<String>,
    /// Whether the oldest kept row links to a recorded prune point.
    pub anchored: bool,
}

/// The id and hash of a row: a saved head, or the point where pruning stopped.
pub type Mark = (i64, String);

/// A check that takes rows one at a time, in id order, so a log of millions of rows is read
/// once and never held in memory.
pub struct Checker<'a> {
    anchor: Option<&'a Mark>,
    saved_head: Option<&'a Mark>,
    out: Verification,
    prev: Option<String>,
    head_seen: Option<bool>,
}

impl<'a> Checker<'a> {
    /// Begin a check. `unchained` is how many rows from before chaining came first: they are not fed.
    pub fn new(anchor: Option<&'a Mark>, saved_head: Option<&'a Mark>, unchained: usize) -> Self {
        Checker { anchor, saved_head, out: Verification { intact: true, checked: 0, unchained, first_break: None, head: None, anchored: false }, prev: None, head_seen: None }
    }

    fn fail(&mut self, id: i64, why: &str) {
        if self.out.first_break.is_none() {
            self.out.first_break = Some(Break { id, why: why.to_string() });
            self.out.intact = false;
        }
    }

    /// Take the next row, from the first chained one on.
    pub fn feed(&mut self, r: &Row) {
        if self.saved_head.is_some_and(|(id, _)| *id == r.id) {
            self.head_seen = Some(self.saved_head.is_some_and(|(_, h)| *h == r.hash));
        }
        // A row without a hash after one with a hash is a row someone blanked to hide an edit.
        if r.hash.is_empty() {
            self.fail(r.id, "a row in the middle of the chain has no hash");
            return;
        }
        self.out.checked += 1;
        self.out.head = Some(r.hash.clone());
        let expected_prev = match &self.prev {
            Some(p) => p.clone(),
            None => match self.anchor {
                // The oldest kept row follows the last one pruned.
                Some((_, h)) if r.prev_hash == *h => {
                    self.out.anchored = true;
                    h.clone()
                }
                // The log was never pruned: the first row starts the chain.
                None => String::new(),
                // Pruned, but this row does not follow the prune point.
                Some((_, h)) => h.clone(),
            },
        };
        if r.prev_hash != expected_prev {
            let first = self.prev.is_none();
            self.fail(
                r.id,
                if first { "the oldest row does not follow the recorded start of the log: rows were removed or changed" } else { "this row does not follow the one before it: a row was removed, inserted or reordered" },
            );
        }
        if link(&r.prev_hash, &r.fields()) != r.hash {
            self.fail(r.id, "this row's content does not match its hash: it was changed");
        }
        self.prev = Some(r.hash.clone());
    }

    /// The end of the log: what was last written must still be there, unless pruning took it.
    pub fn finish(mut self) -> Verification {
        if let Some((id, _)) = self.saved_head {
            let pruned = self.anchor.is_some_and(|(aid, _)| *id <= *aid);
            match self.head_seen {
                Some(true) => {}
                Some(false) => self.fail(*id, "the last recorded row was changed"),
                None if pruned => {}
                None => self.fail(*id, "the last recorded row is gone: the end of the log was removed"),
            }
        }
        self.out
    }
}

/// Check rows in id order against the prune anchor and the saved head.
pub fn verify(rows: &[Row], anchor: Option<&Mark>, saved_head: Option<&Mark>) -> Verification {
    let unchained = rows.iter().take_while(|r| r.hash.is_empty()).count();
    let mut checker = Checker::new(anchor, saved_head, unchained);
    for r in &rows[unchained..] {
        checker.feed(r);
    }
    checker.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, prev: &str, msg: &str) -> Row {
        let mut r = Row {
            id,
            prev_hash: prev.into(),
            hash: String::new(),
            at: format!("2026-01-01T00:00:0{id}Z"),
            actor: "human:cli".into(),
            surface: "cli".into(),
            capability: "site.add".into(),
            tier: "write".into(),
            effect: "write".into(),
            dry_run: false,
            ok: true,
            args: "{}".into(),
            message: msg.into(),
        };
        r.hash = link(&r.prev_hash, &r.fields());
        r
    }

    fn chain(n: i64) -> Vec<Row> {
        let mut rows: Vec<Row> = Vec::new();
        for id in 1..=n {
            let prev = rows.last().map(|r| r.hash.clone()).unwrap_or_default();
            rows.push(row(id, &prev, &format!("did {id}")));
        }
        rows
    }

    fn head(rows: &[Row]) -> Mark {
        let l = rows.last().unwrap();
        (l.id, l.hash.clone())
    }

    #[test]
    fn an_untouched_chain_is_intact() {
        let rows = chain(5);
        let v = verify(&rows, None, Some(&head(&rows)));
        assert!(v.intact && v.checked == 5 && v.unchained == 0 && v.first_break.is_none(), "{v:?}");
        assert_eq!(v.head.as_deref(), Some(rows[4].hash.as_str()));
    }

    #[test]
    fn an_edited_row_is_found_at_that_row() {
        let mut rows = chain(5);
        rows[2].message = "did nothing".into();
        let v = verify(&rows, None, None);
        assert!(!v.intact);
        let b = v.first_break.unwrap();
        assert_eq!(b.id, 3);
        assert!(b.why.contains("changed"), "{}", b.why);
    }

    #[test]
    fn a_removed_middle_row_breaks_the_link_after_it() {
        let mut rows = chain(5);
        rows.remove(2);
        let v = verify(&rows, None, None);
        assert!(!v.intact);
        assert_eq!(v.first_break.unwrap().id, 4);
    }

    #[test]
    fn reordered_rows_are_found() {
        let mut rows = chain(4);
        rows.swap(1, 2);
        assert!(!verify(&rows, None, None).intact);
    }

    #[test]
    fn a_rewritten_row_cannot_be_hidden_by_recomputing_only_its_own_hash() {
        let mut rows = chain(4);
        rows[1].message = "forged".into();
        rows[1].hash = link(&rows[1].prev_hash, &rows[1].fields());
        let v = verify(&rows, None, None);
        assert!(!v.intact, "the next row still points at the old hash");
        assert_eq!(v.first_break.unwrap().id, 3);
    }

    #[test]
    fn cutting_off_the_end_is_found_only_because_the_head_is_kept_apart() {
        let rows = chain(5);
        let saved = head(&rows);
        let cut = &rows[..3];
        assert!(verify(cut, None, None).intact, "a chain alone cannot see its own end cut off");
        let v = verify(cut, None, Some(&saved));
        assert!(!v.intact);
        assert!(v.first_break.unwrap().why.contains("end of the log was removed"));
    }

    #[test]
    fn a_pruned_log_verifies_from_its_anchor_and_a_pruned_and_edited_one_does_not() {
        let rows = chain(6);
        let anchor: Mark = (3, rows[2].hash.clone());
        let kept = rows[3..].to_vec();
        let v = verify(&kept, Some(&anchor), Some(&head(&rows)));
        assert!(v.intact && v.anchored && v.checked == 3, "{v:?}");
        let mut edited = kept.clone();
        edited[0].message = "x".into();
        assert!(!verify(&edited, Some(&anchor), None).intact);
        // Rows pruned without leaving an anchor that matches.
        let wrong: Mark = (3, "0".repeat(64));
        assert!(!verify(&kept, Some(&wrong), None).intact);
    }

    #[test]
    fn rows_from_before_chaining_are_counted_not_blamed() {
        let mut old = row(1, "", "old");
        old.hash.clear();
        old.prev_hash.clear();
        let mut rows = vec![old];
        let first = row(2, "", "first chained");
        rows.push(first);
        let next = row(3, &rows[1].hash.clone(), "second");
        rows.push(next);
        let v = verify(&rows, None, None);
        assert!(v.intact && v.unchained == 1 && v.checked == 2, "{v:?}");
    }

    #[test]
    fn a_blanked_hash_in_the_middle_is_a_break() {
        let mut rows = chain(4);
        rows[2].hash.clear();
        assert!(!verify(&rows, None, None).intact);
    }

    #[test]
    fn text_moved_across_a_field_boundary_hashes_differently() {
        let a = Fields { at: "t", actor: "ab", surface: "c", capability: "x", tier: "r", effect: "r", dry_run: false, ok: true, args: "{}", message: "m" };
        let b = Fields { actor: "a", surface: "bc", ..a.clone() };
        assert_ne!(link("", &a), link("", &b));
    }
}
