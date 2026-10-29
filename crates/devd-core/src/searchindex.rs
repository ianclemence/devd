//! A full-text index of a project's files.
//!
//! `search.query` reads files at query time and cannot rank, stem or find a word
//! inside a large tree quickly. This module keeps an SQLite FTS5 index of the
//! text files of a project, refreshed incrementally from file size and
//! modification time, so a question like "where do we handle the refresh token"
//! is answered by relevance instead of by a literal scan.
//!
//! The index is a cache and nothing else. It lives in its own file
//! (`cache/search.db`), is never the source of an answer about what a file says
//! now, and can be deleted at any time: the next query rebuilds it. Every
//! response says how many files were indexed and how many were skipped, and why,
//! so an empty result is never mistaken for a complete search.

use std::path::Path;

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::observe;
use crate::paths::Paths;
use crate::{DevdError, Result};

/// Largest file indexed, in bytes.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

fn db(e: rusqlite::Error) -> DevdError {
    DevdError::internal(format!("search index: {e}"))
}

/// What a refresh did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Refresh {
    /// Files added or re-read because they changed.
    pub indexed: usize,
    /// Files that were already current.
    pub unchanged: usize,
    /// Files removed from the index because they are gone.
    pub removed: usize,
    /// Files not indexed because they are binary, too large or unreadable.
    pub skipped: usize,
    /// Whether the project had more files than the snapshot limit.
    pub truncated: bool,
}

/// One match.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    /// Path relative to the project.
    pub path: String,
    /// A fragment around the match, the match in square brackets.
    pub snippet: String,
    /// Relevance; lower is better (SQLite's bm25).
    pub rank: f64,
}

/// The index file.
pub struct Index {
    conn: Connection,
}

impl Index {
    /// Open (creating if needed) the index under the devd home.
    pub fn open(paths: &Paths) -> Result<Self> {
        let dir = paths.cache();
        std::fs::create_dir_all(&dir)?;
        Self::open_at(&dir.join("search.db"))
    }

    /// Open an index at an explicit path.
    pub fn open_at(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(db)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                 project TEXT NOT NULL,
                 path    TEXT NOT NULL,
                 mtime   INTEGER NOT NULL,
                 len     INTEGER NOT NULL,
                 rowid_fts INTEGER NOT NULL,
                 PRIMARY KEY (project, path)
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(
                 project UNINDEXED, path, content, tokenize = 'unicode61'
             );",
        )
        .map_err(db)?;
        Ok(Self { conn })
    }

    /// Bring the index of `root` up to date with the files on disk.
    ///
    /// `skip` names files that must never be read into the index, whatever they
    /// are: the caller knows which files hold secrets, and a search index is the
    /// easiest way to hand them to a model by accident.
    pub fn refresh(&mut self, root: &Path, skip: &dyn Fn(&Path) -> bool) -> Result<Refresh> {
        let key = key_of(root);
        let snapshot = observe::take(root);
        let mut report = Refresh {
            truncated: snapshot.truncated,
            ..Refresh::default()
        };

        let known: std::collections::HashMap<String, (i64, i64, i64)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT path, mtime, len, rowid_fts FROM files WHERE project = ?1")
                .map_err(db)?;
            let rows = stmt
                .query_map(params![key], |r| {
                    Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))
                })
                .map_err(db)?;
            rows.filter_map(|r| r.ok()).collect()
        };

        let tx = self.conn.transaction().map_err(db)?;
        for (path, entry) in &snapshot.files {
            // Nanoseconds do not fit a 64-bit signed column for far-future dates;
            // milliseconds are plenty to tell a file changed.
            let mtime = (entry.mtime / 1_000_000) as i64;
            let len = entry.len as i64;
            if let Some((m, l, _)) = known.get(path) {
                if *m == mtime && *l == len {
                    report.unchanged += 1;
                    continue;
                }
            }
            let full = root.join(path);
            let text = if skip(&full) {
                None
            } else {
                read_text(&full, entry.len)
            };
            let Some(text) = text else {
                report.skipped += 1;
                // A file that stopped being indexable must not keep stale content.
                if let Some((_, _, rowid)) = known.get(path) {
                    tx.execute("DELETE FROM fts WHERE rowid = ?1", params![rowid])
                        .map_err(db)?;
                    tx.execute(
                        "DELETE FROM files WHERE project = ?1 AND path = ?2",
                        params![key, path],
                    )
                    .map_err(db)?;
                }
                continue;
            };
            if let Some((_, _, rowid)) = known.get(path) {
                tx.execute("DELETE FROM fts WHERE rowid = ?1", params![rowid])
                    .map_err(db)?;
            }
            tx.execute(
                "INSERT INTO fts (project, path, content) VALUES (?1, ?2, ?3)",
                params![key, path, text],
            )
            .map_err(db)?;
            let rowid = tx.last_insert_rowid();
            tx.execute(
                "INSERT OR REPLACE INTO files (project, path, mtime, len, rowid_fts) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![key, path, mtime, len, rowid],
            )
            .map_err(db)?;
            report.indexed += 1;
        }
        // Files that were indexed and are no longer in the project. A truncated
        // snapshot does not know about every file, so nothing is removed on its
        // account.
        if !snapshot.truncated {
            for (path, (_, _, rowid)) in &known {
                if !snapshot.files.contains_key(path) {
                    tx.execute("DELETE FROM fts WHERE rowid = ?1", params![rowid])
                        .map_err(db)?;
                    tx.execute(
                        "DELETE FROM files WHERE project = ?1 AND path = ?2",
                        params![key, path],
                    )
                    .map_err(db)?;
                    report.removed += 1;
                }
            }
        }
        tx.commit().map_err(db)?;
        Ok(report)
    }

    /// Files of `root` that match `query`, best first.
    ///
    /// Every word must appear; the last word also matches as a prefix, so a
    /// query typed as the person is still typing it finds something.
    pub fn query(&self, root: &Path, query: &str, limit: usize) -> Result<Vec<Hit>> {
        let Some(expression) = match_expression(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, snippet(fts, 2, '[', ']', '...', 14), bm25(fts, 0.0, 4.0, 1.0)
                 FROM fts WHERE fts MATCH ?1 AND project = ?2
                 ORDER BY 3 LIMIT ?3",
            )
            .map_err(db)?;
        let rows = stmt
            .query_map(params![expression, key_of(root), limit as i64], |r| {
                Ok(Hit {
                    path: r.get(0)?,
                    snippet: r.get::<_, String>(1)?.replace(['\r', '\n'], " "),
                    rank: r.get(2)?,
                })
            })
            .map_err(db)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// How many files of `root` are indexed.
    pub fn count(&self, root: &Path) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE project = ?1",
                params![key_of(root)],
                |r| r.get(0),
            )
            .map_err(db)?;
        Ok(n as usize)
    }
}

fn key_of(root: &Path) -> String {
    root.to_string_lossy().replace('\\', "/").to_lowercase()
}

/// An FTS5 `MATCH` expression for what a person typed, or `None` when it has no
/// searchable word. Each word is quoted, so punctuation and FTS operators in the
/// query are text, never syntax.
pub fn match_expression(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let last = words.len().checked_sub(1)?;
    Some(
        words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                if i == last {
                    format!("\"{w}\"*")
                } else {
                    format!("\"{w}\"")
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn read_text(path: &Path, len: u64) -> Option<String> {
    if len > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    // A NUL in the first block is the usual sign of a binary file.
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, tempfile::TempDir) {
        (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

    #[test]
    fn a_word_is_found_by_relevance_with_a_snippet_around_it() {
        let (proj, home) = project();
        std::fs::write(
            proj.path().join("auth.rs"),
            "fn refresh_token(session: &Session) { renew(session) }\n",
        )
        .unwrap();
        std::fs::write(proj.path().join("notes.md"), "nothing about it here\n").unwrap();
        let mut index = Index::open_at(&home.path().join("s.db")).unwrap();
        let report = index.refresh(proj.path(), &|_| false).unwrap();
        assert_eq!(report.indexed, 2);

        let hits = index.query(proj.path(), "refresh token", 10).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].path, "auth.rs");
        assert!(hits[0].snippet.contains('['), "{}", hits[0].snippet);

        // The last word matches as a prefix, as it would while being typed.
        assert_eq!(index.query(proj.path(), "refresh tok", 10).unwrap().len(), 1);
        assert!(index.query(proj.path(), "absent", 10).unwrap().is_empty());
    }

    #[test]
    fn a_refresh_reads_only_what_changed_and_forgets_what_is_gone() {
        let (proj, home) = project();
        std::fs::write(proj.path().join("a.txt"), "alpha").unwrap();
        std::fs::write(proj.path().join("b.txt"), "beta").unwrap();
        let mut index = Index::open_at(&home.path().join("s.db")).unwrap();
        index.refresh(proj.path(), &|_| false).unwrap();

        let again = index.refresh(proj.path(), &|_| false).unwrap();
        assert_eq!((again.indexed, again.unchanged), (0, 2));

        std::fs::write(proj.path().join("a.txt"), "alpha changed to gamma").unwrap();
        std::fs::remove_file(proj.path().join("b.txt")).unwrap();
        let third = index.refresh(proj.path(), &|_| false).unwrap();
        assert_eq!((third.indexed, third.unchanged, third.removed), (1, 0, 1));

        assert_eq!(index.query(proj.path(), "gamma", 5).unwrap().len(), 1);
        assert!(index.query(proj.path(), "beta", 5).unwrap().is_empty());
        assert_eq!(index.count(proj.path()).unwrap(), 1);
    }

    #[test]
    fn binary_and_oversize_files_are_skipped_and_counted() {
        let (proj, home) = project();
        std::fs::write(proj.path().join("image.bin"), [0u8, 1, 2, 0, 3]).unwrap();
        std::fs::write(proj.path().join("ok.txt"), "searchable").unwrap();
        let mut index = Index::open_at(&home.path().join("s.db")).unwrap();
        let report = index.refresh(proj.path(), &|_| false).unwrap();
        assert_eq!((report.indexed, report.skipped), (1, 1));
    }

    #[test]
    fn a_file_the_caller_marks_secret_is_never_read_into_the_index() {
        let (proj, home) = project();
        std::fs::write(proj.path().join(".env"), "API_KEY=hunter2").unwrap();
        std::fs::write(proj.path().join("app.txt"), "ordinary text").unwrap();
        let mut index = Index::open_at(&home.path().join("s.db")).unwrap();
        let report = index
            .refresh(proj.path(), &|p| p.file_name().is_some_and(|n| n == ".env"))
            .unwrap();
        assert_eq!((report.indexed, report.skipped), (1, 1));
        assert!(index.query(proj.path(), "hunter2", 5).unwrap().is_empty());
        assert_eq!(index.query(proj.path(), "ordinary", 5).unwrap().len(), 1);
    }

    #[test]
    fn projects_do_not_see_each_others_files() {
        let (one, home) = project();
        let two = tempfile::tempdir().unwrap();
        std::fs::write(one.path().join("a.txt"), "shared word").unwrap();
        std::fs::write(two.path().join("b.txt"), "shared word").unwrap();
        let mut index = Index::open_at(&home.path().join("s.db")).unwrap();
        index.refresh(one.path(), &|_| false).unwrap();
        index.refresh(two.path(), &|_| false).unwrap();
        let hits = index.query(one.path(), "shared", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "a.txt");
    }

    #[test]
    fn fts_syntax_in_a_query_is_text_not_a_command() {
        assert_eq!(
            match_expression("auth AND (token OR \"x\")").as_deref(),
            Some("\"auth\" \"AND\" \"token\" \"OR\" \"x\"*")
        );
        assert_eq!(match_expression("***"), None);
        assert_eq!(match_expression("   "), None);
        let (proj, home) = project();
        std::fs::write(proj.path().join("a.txt"), "token").unwrap();
        let mut index = Index::open_at(&home.path().join("s.db")).unwrap();
        index.refresh(proj.path(), &|_| false).unwrap();
        assert!(index.query(proj.path(), "NEAR( \" ' ) *", 5).is_ok());
    }
}
