//! Fuzzy matching for the command menu and every filter box.
//!
//! A query matches a candidate when its characters appear in the candidate in
//! order. The score says how good the match is, so that typing `gs` puts
//! "Git: Status" above "Diagnostics": a hit at the start of a word is worth more
//! than one in the middle, consecutive hits are worth more than scattered ones,
//! and a shorter candidate wins a tie.

/// A candidate that matched, with where it matched.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// Higher is better. Only comparable between matches of the same query.
    pub score: i32,
    /// Byte offsets into the candidate of each matched character, for highlighting.
    pub positions: Vec<usize>,
}

const HIT: i32 = 16;
const WORD_START: i32 = 24;
const CONSECUTIVE: i32 = 18;
const FIRST_CHAR: i32 = 12;
const EXACT_CASE: i32 = 1;
const GAP: i32 = 2;

/// Match `query` against `candidate`. An empty query matches everything with score 0.
pub fn matches(query: &str, candidate: &str) -> Option<Match> {
    let needle: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
    if needle.is_empty() {
        return Some(Match {
            score: 0,
            positions: Vec::new(),
        });
    }
    let hay: Vec<(usize, char)> = candidate.char_indices().collect();
    if needle.len() > hay.len() {
        return None;
    }
    // best[i][j]: the best score matching needle[..=i] with needle[i] at hay[j].
    let (n, h) = (needle.len(), hay.len());
    let mut best = vec![i32::MIN; n * h];
    let mut from = vec![usize::MAX; n * h];
    for i in 0..n {
        let want = needle[i];
        for j in i..h {
            let (_, got) = hay[j];
            if !same(want, got) {
                continue;
            }
            let mut score = HIT + if want == got { EXACT_CASE } else { 0 };
            if j == 0 {
                score += FIRST_CHAR + WORD_START;
            } else if starts_word(hay[j - 1].1, got) {
                score += WORD_START;
            }
            if i == 0 {
                best[j] = score - (j as i32).min(8) * GAP / 2;
                continue;
            }
            let mut top = i32::MIN;
            let mut top_k = usize::MAX;
            for k in (i - 1)..j {
                let prev = best[(i - 1) * h + k];
                if prev == i32::MIN {
                    continue;
                }
                let gap = (j - k - 1) as i32;
                let joined = if gap == 0 {
                    CONSECUTIVE
                } else {
                    -(gap.min(12) * GAP)
                };
                if prev + joined > top {
                    top = prev + joined;
                    top_k = k;
                }
            }
            if top != i32::MIN {
                best[i * h + j] = top + score;
                from[i * h + j] = top_k;
            }
        }
    }
    let last = (n - 1) * h;
    let (end, score) = (0..h)
        .map(|j| (j, best[last + j]))
        .filter(|(_, s)| *s != i32::MIN)
        .max_by_key(|(_, s)| *s)?;
    let mut positions = vec![0; n];
    let mut j = end;
    for i in (0..n).rev() {
        positions[i] = hay[j].0;
        if i > 0 {
            j = from[i * h + j];
        }
    }
    Some(Match {
        score: score - (h as i32) / 4,
        positions,
    })
}

fn same(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Whether `current` begins a word given the character before it.
fn starts_word(previous: char, current: char) -> bool {
    !previous.is_alphanumeric() && current.is_alphanumeric()
        || previous.is_lowercase() && current.is_uppercase()
        || previous.is_alphabetic() && current.is_numeric()
}

/// Rank `items` against `query`, best first. Items that do not match are dropped.
///
/// With an empty query the items come back in the order given, which lets the
/// caller put recent or suggested entries first.
pub fn rank<'a, T>(query: &str, items: &'a [T], text: impl Fn(&T) -> &str) -> Vec<(&'a T, Match)> {
    let mut out: Vec<(usize, &T, Match)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| matches(query, text(item)).map(|m| (i, item, m)))
        .collect();
    out.sort_by(|a, b| b.2.score.cmp(&a.2.score).then(a.0.cmp(&b.0)));
    out.into_iter().map(|(_, item, m)| (item, m)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order<'a>(query: &str, items: &'a [&'a str]) -> Vec<&'a str> {
        rank(query, items, |s| s)
            .into_iter()
            .map(|(s, _)| *s)
            .collect()
    }

    #[test]
    fn characters_must_appear_in_order() {
        assert!(matches("gst", "Git: Status").is_some());
        assert!(matches("tsg", "Git: Status").is_none());
        assert!(matches("", "anything").is_some());
        assert!(matches("longer than the text", "short").is_none());
    }

    #[test]
    fn word_starts_beat_hits_inside_a_word() {
        let items = ["Diagnostics", "Git: Status", "Go to Sites"];
        assert_eq!(order("gs", &items)[..2], ["Git: Status", "Go to Sites"]);
    }

    #[test]
    fn a_run_of_letters_beats_the_same_letters_scattered() {
        let items = ["Start nginx", "Sites: new", "Restart nginx"];
        assert_eq!(order("ngin", &items)[0], "Start nginx");
        assert_eq!(
            order("site", &["Sign in to the editor", "Sites"])[0],
            "Sites"
        );
    }

    #[test]
    fn the_positions_point_at_the_matched_characters() {
        let m = matches("gs", "Git: Status").unwrap();
        assert_eq!(m.positions, [0, 5]);
        let unicode = matches("é", "café").unwrap();
        assert_eq!(&"café"[unicode.positions[0]..], "é");
    }

    #[test]
    fn an_empty_query_keeps_the_given_order() {
        assert_eq!(order("", &["b", "a", "c"]), ["b", "a", "c"]);
    }

    #[test]
    fn camel_case_and_separators_start_words() {
        assert!(matches("fb", "fooBar").unwrap().score > matches("fb", "fobar").unwrap().score);
        assert!(
            matches("nu", "nairobi_unwind").unwrap().score
                > matches("nu", "nautilus").unwrap().score - 40
        );
    }
}
