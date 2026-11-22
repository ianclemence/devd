//! What the theme cards say about themselves, read without a window.
//!
//! The card's picture is geometry; what a person is told about a theme is text. These keep the text honest:
//! five cards, one per built-in theme, each with a name, how it feels and what is behind it, and none of them
//! described with the words "light" and "dark", which name a mode rather than a look.

use super::appearance::CHARACTER;
use crate::theme::Theme;

#[test]
fn every_builtin_theme_has_one_card() {
    let cards: Vec<&str> = CHARACTER.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(
        Theme::ORDER.to_vec(),
        cards,
        "the picker and the theme table must list the same themes in the same order"
    );
}

#[test]
fn a_card_says_how_the_theme_feels_and_what_is_behind_it() {
    for (id, feel, scene) in CHARACTER {
        assert!(!feel.trim().is_empty(), "{id} has no character");
        assert!(scene.trim().ends_with('.'), "{id}'s description is not a sentence");
        let text = format!("{feel} {scene}").to_lowercase();
        assert!(
            !text.contains("light mode") && !text.contains("dark mode") && !text.contains(" dark"),
            "{id} is described by a mode: {text}"
        );
    }
}
