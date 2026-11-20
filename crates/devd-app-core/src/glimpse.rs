//! The one faint line that says a model is thinking.
//!
//! Reasoning reaches the window as a stream of pieces that was never written to be read on one
//! line: it carries markdown emphasis, code fences and line breaks, and any tail of it begins and
//! ends mid-word. Shown raw beside "Thinking", it reads as something broken rather than as
//! progress - the window really did show `...choice,** The spec asks: ort for desktop, ONNX Runti`.
//!
//! So a glimpse is the thought's **last complete sentence**, stripped to one line, and a long one
//! is cut at a word boundary with a leading ellipsis. When no sentence has ended yet, the tail of
//! the thought is used the same way, without its half first word.

/// The longest glimpse worth one faint line beside a label.
const LONGEST: usize = 160;

/// The glimpse of a thought, given what was shown before and the piece that just arrived.
pub fn of(before: &str, piece: &str) -> String {
    let mut raw = String::with_capacity(before.len() + piece.len() + 1);
    raw.push_str(before);
    raw.push_str(piece);
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            '*' | '`' | '#' | '_' => ' ',
            '\n' | '\r' | '\t' => ' ',
            other => other,
        })
        .collect();
    let one_line = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    // The last sentence that actually ended; a thought with none keeps its tail.
    let after_end = [". ", "! ", "? "]
        .iter()
        .filter_map(|end| one_line.rfind(end).map(|at| at + end.len()))
        .max();
    let mut text = match after_end {
        Some(at) => one_line[at..].trim().to_string(),
        None => match one_line.split_once(' ') {
            // Drop the first word: the tail of a stream usually starts in the middle of one.
            Some((_, rest)) => rest.trim_start().to_string(),
            None => String::new(),
        },
    };
    if text.chars().count() > LONGEST {
        let head: String = text.chars().take(LONGEST).collect();
        let at = head.rfind(' ').unwrap_or(head.len());
        text = format!("…{}", head[..at].trim_end());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::of;

    #[test]
    fn the_glimpse_is_the_last_complete_sentence_on_one_line() {
        let said = of(
            "Let me look at the spec. The spec asks: ",
            "**support for desktop**\nONNX Runtime",
        );
        assert!(!said.contains('*'), "{said}");
        assert!(!said.contains('\n'), "{said}");
        assert!(said.starts_with("The spec asks:"), "{said}");
        assert!(said.ends_with("ONNX Runtime"), "{said}");
    }

    #[test]
    fn a_thought_with_no_full_stop_drops_its_half_first_word() {
        assert_eq!(of("", "ort for desktop, ONNX Runti"), "for desktop, ONNX Runti");
    }

    #[test]
    fn a_long_sentence_is_cut_at_a_word_and_marked() {
        let said = of("", &"word ".repeat(80));
        assert!(said.starts_with('…'), "{said}");
        assert!(said.chars().count() <= 161, "{}", said.chars().count());
        assert!(!said.ends_with("wor"), "a cut word is not shown: {said}");
    }

    #[test]
    fn nothing_arrives_means_nothing_is_shown() {
        assert_eq!(of("", ""), "");
    }
}
