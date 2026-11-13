//! What a key press means to a shell.
//!
//! A terminal does not receive "keys"; it receives the bytes a keyboard would have sent down a serial line.
//! Enter is a carriage return, Backspace is DEL, an arrow is an escape sequence, Ctrl+C is the byte 3. This
//! is that table, with no window in it, so it can be tested.

/// A key press, as the window reports it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Press<'a> {
    /// The key's name: `enter`, `a`, `up`, `f5`…
    pub key: &'a str,
    /// The text the key types, when it types any (`A` for shift+a, `é`).
    pub text: Option<&'a str>,
    /// Ctrl was held.
    pub ctrl: bool,
    /// Alt was held.
    pub alt: bool,
    /// Shift was held.
    pub shift: bool,
}

/// What to send the shell for a key, or nothing when the key is not the shell's (a bare modifier).
pub fn bytes(press: Press<'_>) -> Option<String> {
    let Press { key, text, ctrl, alt, shift } = press;
    let named = match key {
        "enter" => Some("\r"),
        "backspace" => Some("\u{7f}"),
        "tab" if shift => Some("\u{1b}[Z"),
        "tab" => Some("\t"),
        "escape" => Some("\u{1b}"),
        "up" => Some("\u{1b}[A"),
        "down" => Some("\u{1b}[B"),
        "right" => Some("\u{1b}[C"),
        "left" => Some("\u{1b}[D"),
        "home" => Some("\u{1b}[H"),
        "end" => Some("\u{1b}[F"),
        "pageup" => Some("\u{1b}[5~"),
        "pagedown" => Some("\u{1b}[6~"),
        "delete" => Some("\u{1b}[3~"),
        "insert" => Some("\u{1b}[2~"),
        "f1" => Some("\u{1b}OP"),
        "f2" => Some("\u{1b}OQ"),
        "f3" => Some("\u{1b}OR"),
        "f4" => Some("\u{1b}OS"),
        "f5" => Some("\u{1b}[15~"),
        "f6" => Some("\u{1b}[17~"),
        "f7" => Some("\u{1b}[18~"),
        "f8" => Some("\u{1b}[19~"),
        "f9" => Some("\u{1b}[20~"),
        "f10" => Some("\u{1b}[21~"),
        "f11" => Some("\u{1b}[23~"),
        "f12" => Some("\u{1b}[24~"),
        _ => None,
    };
    if let Some(sequence) = named {
        // Alt in front of a key is ESC in front of what it sends.
        return Some(if alt { format!("\u{1b}{sequence}") } else { sequence.to_string() });
    }
    // Ctrl and a letter is that letter's position in the alphabet: Ctrl+C is 3, Ctrl+D is 4.
    if ctrl {
        let mut chars = key.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else { return None };
        return match c {
            'a'..='z' => Some(((c as u8 - b'a' + 1) as char).to_string()),
            '[' => Some("\u{1b}".to_string()),
            '\\' => Some("\u{1c}".to_string()),
            ']' => Some("\u{1d}".to_string()),
            ' ' | '@' | '2' => Some("\0".to_string()),
            _ => None,
        };
    }
    if key == "space" {
        return Some(if alt { "\u{1b} ".to_string() } else { " ".to_string() });
    }
    let typed = text.filter(|t| !t.is_empty())?;
    Some(if alt { format!("\u{1b}{typed}") } else { typed.to_string() })
}

/// Text that is pasted, as a shell wants it: line breaks as the Enter key sends them.
pub fn pasted(text: &str) -> String {
    text.replace("\r\n", "\r").replace('\n', "\r")
}

/// A terminal's screen as lines: trailing blank lines dropped, so an empty shell is empty and a short one
/// is not padded.
pub fn lines(screen: &str) -> Vec<&str> {
    let mut all: Vec<&str> = screen.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    while all.last().is_some_and(|l| l.trim().is_empty()) {
        all.pop();
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: &str) -> Press<'_> {
        Press { key, ..Press::default() }
    }

    #[test]
    fn the_keys_a_shell_is_driven_by_send_what_a_keyboard_would() {
        assert_eq!(bytes(key("enter")).as_deref(), Some("\r"));
        assert_eq!(bytes(key("backspace")).as_deref(), Some("\u{7f}"));
        assert_eq!(bytes(key("up")).as_deref(), Some("\u{1b}[A"));
        assert_eq!(bytes(Press { key: "tab", shift: true, ..Press::default() }).as_deref(), Some("\u{1b}[Z"));
        assert_eq!(bytes(Press { key: "c", ctrl: true, ..Press::default() }).as_deref(), Some("\u{3}"));
        assert_eq!(bytes(Press { key: "d", ctrl: true, ..Press::default() }).as_deref(), Some("\u{4}"));
        assert_eq!(bytes(Press { key: "left", alt: true, ..Press::default() }).as_deref(), Some("\u{1b}\u{1b}[D"));
    }

    #[test]
    fn typing_sends_the_text_the_key_types_and_a_bare_modifier_sends_nothing() {
        assert_eq!(bytes(Press { key: "a", text: Some("A"), shift: true, ..Press::default() }).as_deref(), Some("A"));
        assert_eq!(bytes(Press { key: "e", text: Some("é"), ..Press::default() }).as_deref(), Some("é"));
        assert_eq!(bytes(key("space")).as_deref(), Some(" "));
        assert_eq!(bytes(key("shift")), None);
        assert_eq!(bytes(key("control")), None);
    }

    #[test]
    fn pasted_text_ends_its_lines_the_way_enter_does() {
        assert_eq!(pasted("a\r\nb\nc"), "a\rb\rc");
    }

    #[test]
    fn a_screen_has_no_trailing_blank_lines() {
        assert_eq!(lines("one\r\ntwo\r\n\r\n  \n"), vec!["one", "two"]);
        assert!(lines("").is_empty());
    }
}
