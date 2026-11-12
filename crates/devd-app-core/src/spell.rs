//! Spelling, for the message box and anywhere else the person types prose.
//!
//! No spell checker is the same on three operating systems, and a window that
//! calls each system's own would behave three ways. This one is plain Rust: it
//! finds the words in a piece of text, decides which are probably mistakes, and
//! offers corrections, the same everywhere.
//!
//! The rule that keeps it quiet is that a word is flagged only when it is not a
//! known word *and* a known word lies one slip away from it (a letter dropped,
//! added, changed or swapped). "teh" is flagged because "the" is next to it;
//! "devd" or "kubectl" is not, because nothing is. A dictionary this window
//! ships with is small, and under that rule it can be small without crying
//! wolf at every name and every word it has not met. A fuller dictionary found
//! on the machine, or the person's own words, makes it catch more, never less
//! sure.
//!
//! What is never checked: anything in code, paths, addresses, handles, words
//! with digits or capitals inside them, and acronyms.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// What the window ships with: the common words of ordinary writing, and the
/// vocabulary of devd's own documentation.
const BUNDLED: &str = concat!(include_str!("../data/common.txt"), "\n", include_str!("../data/words.txt"));

/// A set of known words.
#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    words: HashSet<String>,
}

impl Dictionary {
    /// The dictionary the window ships with.
    pub fn bundled() -> Self {
        Self::from_text(BUNDLED)
    }

    /// Words from text: one per line (a Hunspell `.dic` file's `word/flags`
    /// lines and its count line are understood), or separated by spaces.
    pub fn from_text(text: &str) -> Self {
        let mut dictionary = Self::default();
        dictionary.add_text(text);
        dictionary
    }

    /// Add more words.
    pub fn add_text(&mut self, text: &str) {
        for token in text.split_whitespace() {
            // `colour/MS` is the word and its Hunspell flags.
            let word = token.split('/').next().unwrap_or(token);
            if word.chars().all(|c| c.is_alphabetic() || c == '\'') && word.chars().count() >= 2 {
                self.words.insert(word.to_lowercase());
            }
        }
    }

    /// Add one word, as the person does when they tell the window a word is fine.
    pub fn add(&mut self, word: &str) {
        self.add_text(word);
    }

    /// Add a dictionary file, if it can be read.
    pub fn add_file(&mut self, path: &Path) -> bool {
        match std::fs::read(path) {
            Ok(bytes) => {
                self.add_text(&String::from_utf8_lossy(&bytes));
                true
            }
            Err(_) => false,
        }
    }

    /// The window's dictionary on this machine: the bundled one, any fuller one
    /// the system keeps, and the person's own words from `personal`.
    pub fn for_this_machine(personal: &Path) -> Self {
        let mut dictionary = Self::bundled();
        for path in system_dictionaries() {
            dictionary.add_file(&path);
        }
        dictionary.add_file(personal);
        dictionary
    }

    /// How many words are known.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether no word is known.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Whether a word is known. Case does not matter, and a possessive or a
    /// plural of a known word is known.
    pub fn knows(&self, word: &str) -> bool {
        let lower = word.to_lowercase();
        if self.words.contains(&lower) {
            return true;
        }
        // A contraction is its parts: it's, don't, we'll, they're.
        if let Some((head, tail)) = lower.split_once('\'') {
            let joined = format!("{head}{tail}");
            if matches!(tail, "s" | "t" | "re" | "ve" | "ll" | "d" | "m") && (self.words.contains(head) || self.words.contains(&joined)) {
                return true;
            }
        }
        let stem = lower.strip_suffix("'s").or_else(|| lower.strip_suffix('s')).or_else(|| lower.strip_suffix("ed")).or_else(|| lower.strip_suffix("ing"));
        stem.is_some_and(|stem| stem.chars().count() >= 3 && (self.words.contains(stem) || self.words.contains(&format!("{stem}e"))))
    }

    /// Known words one slip away from `word`, nearest first.
    pub fn near(&self, word: &str) -> Vec<String> {
        let lower = word.to_lowercase();
        let chars: Vec<char> = lower.chars().collect();
        let mut found: Vec<String> = Vec::new();
        let try_word = |candidate: String, found: &mut Vec<String>| {
            if candidate != lower && self.knows(&candidate) && !found.contains(&candidate) {
                found.push(candidate);
            }
        };
        // Swapped neighbours first: the commonest slip of the hands.
        for i in 0..chars.len().saturating_sub(1) {
            let mut swapped = chars.clone();
            swapped.swap(i, i + 1);
            try_word(swapped.into_iter().collect(), &mut found);
        }
        for i in 0..chars.len() {
            let mut dropped = chars.clone();
            dropped.remove(i);
            try_word(dropped.into_iter().collect(), &mut found);
        }
        for i in 0..=chars.len() {
            for letter in 'a'..='z' {
                let mut added = chars.clone();
                added.insert(i, letter);
                try_word(added.into_iter().collect(), &mut found);
            }
        }
        for i in 0..chars.len() {
            for letter in 'a'..='z' {
                let mut changed = chars.clone();
                changed[i] = letter;
                try_word(changed.into_iter().collect(), &mut found);
            }
        }
        found
    }

    /// Corrections for a word, in the case it was typed in. At most `limit`.
    pub fn suggest(&self, word: &str, limit: usize) -> Vec<String> {
        let first_upper = word.chars().next().is_some_and(char::is_uppercase);
        let all_upper = word.chars().count() > 1 && word.chars().all(|c| !c.is_lowercase());
        self.near(word)
            .into_iter()
            .take(limit)
            .map(|s| {
                if all_upper {
                    s.to_uppercase()
                } else if first_upper {
                    let mut chars = s.chars();
                    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
                } else {
                    s
                }
            })
            .collect()
    }

    /// The byte ranges in `text` of words that are probably mistakes.
    pub fn check(&self, text: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        for (range, word) in words_in(text) {
            if !self.knows(&word) && !self.near(&word).is_empty() {
                out.push(range);
            }
        }
        out
    }
}

/// Fuller dictionaries operating systems and editors keep in known places.
fn system_dictionaries() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let fixed = [
        "/usr/share/dict/words",
        "/usr/share/dict/american-english",
        "/usr/share/hunspell/en_US.dic",
        "/usr/share/myspell/en_US.dic",
        "/Library/Spelling/en_US.dic",
        "/System/Library/Spelling/en_US.dic",
    ];
    found.extend(fixed.iter().map(PathBuf::from).filter(|p| p.is_file()));
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        for relative in ["Library/Spelling/en_US.dic", ".config/enchant/en_US.dic", ".local/share/hunspell/en_US.dic"] {
            let path = home.join(relative);
            if path.is_file() {
                found.push(path);
            }
        }
    }
    found
}

/// The words of ordinary prose in `text`, with where each is.
///
/// A word is a run of letters, with an apostrophe allowed inside it. Whatever
/// is not ordinary prose is skipped: see the module comment.
pub fn words_in(text: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut in_code = false;
    let mut in_fence = false;
    let bytes = text.as_bytes();
    let mut iter = text.char_indices().peekable();
    while let Some((start, c)) = iter.next() {
        if c == '`' {
            if text[start..].starts_with("```") {
                in_fence = !in_fence;
                for _ in 0..2 {
                    iter.next();
                }
            } else {
                in_code = !in_code;
            }
            continue;
        }
        if in_code || in_fence || !c.is_alphabetic() {
            continue;
        }
        // A word that starts at `start`: take letters, and apostrophes between letters.
        let mut end = start + c.len_utf8();
        let mut glued = false;
        while let Some(&(i, n)) = iter.peek() {
            if n.is_alphabetic() {
                end = i + n.len_utf8();
                iter.next();
            } else if n == '\'' && text[i + 1..].chars().next().is_some_and(char::is_alphabetic) {
                end = i + 1;
                iter.next();
            } else if n.is_ascii_digit() || n == '_' {
                glued = true;
                end = i + 1;
                iter.next();
            } else {
                break;
            }
        }
        let word = &text[start..end];
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        // A capital in the middle of a sentence is a name: Theo, Nairobi, Laravel.
        // One that opens a sentence is just a word that happens to be capitalised.
        let opens_sentence = text[..start].trim_end().chars().next_back().map_or(true, |p| matches!(p, '.' | '!' | '?' | ':' | '\n' | '#' | '*' | '-' | '>')) || text[..start].ends_with('\n');
        let is_name = c.is_uppercase() && !opens_sentence;
        let skip = glued
            || is_name
            || word.chars().count() < 3
            // Capitals inside a word (camelCase, iPhone) or all capitals (an acronym).
            || word.chars().skip(1).any(char::is_uppercase)
            // A path, an address, a handle, a flag, a tag, a file extension, a method.
            || before.is_some_and(|b| matches!(b, '/' | '\\' | '.' | '_' | '@' | '#' | ':' | '-' | '$' | '%' | '<' | '='))
            || after.is_some_and(|a| matches!(a, '/' | '\\' | '_' | '@' | ':' | '('))
            || (after == Some('.') && bytes.get(end + 1).is_some_and(|b| b.is_ascii_alphabetic()));
        if !skip {
            out.push((start..end, word.to_string()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> Dictionary {
        Dictionary::from_text("the project because receive database however color colour")
    }

    #[test]
    fn a_slip_of_the_hands_is_flagged_and_corrected() {
        let d = small();
        for (typed, meant) in [("teh", "the"), ("projct", "project"), ("becuase", "because"), ("recieve", "receive"), ("databse", "database"), ("howver", "however")] {
            assert_eq!(d.check(typed), vec![0..typed.len()], "{typed} is flagged");
            assert_eq!(d.suggest(typed, 3).first().map(String::as_str), Some(meant), "{typed}");
        }
    }

    #[test]
    fn a_word_the_dictionary_has_not_met_is_left_alone() {
        let d = small();
        for text in ["kubectl", "devd", "Nairobi", "ocean", "gpui", "ask Theo"] {
            assert!(d.check(text).is_empty(), "{text} is not a slip of anything known");
        }
    }

    #[test]
    fn plurals_possessives_and_case_are_not_mistakes() {
        let d = small();
        assert!(d.check("Projects project's PROJECT The databases").is_empty());
        assert_eq!(d.suggest("Teh", 1), ["The"], "a capital stays");
        assert_eq!(d.suggest("TEH", 1), ["THE"]);
    }

    #[test]
    fn code_paths_addresses_and_identifiers_are_never_checked() {
        let d = small();
        let text = "run `teh` plus see src/projct/teh.rs and https://projct.test/teh or @teh #teh --teh camelTeh projct_2 TEH snake_projct ```\nteh projct\n```";
        assert_eq!(d.check(text), Vec::<Range<usize>>::new(), "{:?}", words_in(text));
        // …and prose beside them still is.
        let beside = "see `code` but teh rest";
        let flagged = d.check(beside);
        assert_eq!(flagged.len(), 1);
        assert_eq!(&beside[flagged[0].clone()], "teh");
    }

    #[test]
    fn ranges_are_bytes_so_they_slice_text_with_other_scripts_and_emoji() {
        let d = small();
        let text = "café ☕ teh näive teh";
        let flagged = d.check(text);
        assert_eq!(flagged.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(), ["teh", "teh"]);
    }

    #[test]
    fn a_hunspell_file_and_the_persons_own_words_are_understood() {
        let mut d = Dictionary::from_text("3\ncolour/MS\nneighbour/SM\nteh/X\n");
        assert!(d.knows("colour") && d.knows("neighbours"));
        // The person says a word is fine: it stops being flagged, and stops being a candidate to flag others.
        assert!(d.knows("teh"));
        d.add("projct");
        assert!(d.check("projct").is_empty());
    }

    #[test]
    fn the_bundled_dictionary_catches_everyday_slips_without_flagging_everyday_words() {
        let d = Dictionary::bundled();
        assert!(d.len() > 2_500, "{}", d.len());
        let fine = "Please check the project and tell me whether the database migration finished, because the tests failed yesterday. It's the Laravel app, not the kubectl one.";
        assert_eq!(d.check(fine).iter().map(|r| &fine[r.clone()]).collect::<Vec<_>>(), Vec::<&str>::new());
        let slips = "I recieve teh message becuase of an eror";
        let flagged: Vec<&str> = d.check(slips).iter().map(|r| &slips[r.clone()]).collect();
        assert!(flagged.contains(&"recieve") && flagged.contains(&"teh") && flagged.contains(&"becuase"), "{flagged:?}");
    }
}
