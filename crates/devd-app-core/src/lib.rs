//! The native devd application without its window.
//!
//! `devd-app` draws; this crate decides. Everything here is ordinary Rust with
//! no GPU, no event loop and no devd process behind it, so each behaviour of the
//! window has a test that runs in milliseconds:
//!
//! | Module | What it owns |
//! | --- | --- |
//! | [`sync`] | The cache of what devd has told the window, kept true by events, with optimistic writes |
//! | [`command`] | Every action the window has, declared once for the menu, the keymap and agents |
//! | [`fuzzy`] | Matching and ranking for the command menu and filter boxes |
//! | [`links`] | The web addresses and files in a text that can be clicked |
//! | [`markdown`] | Agent replies, parsed into blocks the window draws itself |
//! | [`diff`] | Git's diffs and before/after comparisons, down to the changed word |
//! | [`motion`] | Springs and curves that can be interrupted mid-flight |
//! | [`presence`] | Who is where and doing what, derived from devd's own records |
//! | [`nav`] | Where the window is, as text a link or an agent can name |
//! | [`order`] | The arithmetic of drag and drop |
//! | [`toolname`] | What an agent did, in a person's words instead of a capability id |
//! | [`spell`] | Finding and correcting slips in what the person types |
//! | [`status`] | What a state means: its colour, its word, whether it needs the person |
//!
//! The split is the same rule the rest of devd follows: one place decides, and
//! the surfaces are views of it.

pub mod command;
pub mod diff;
pub mod fuzzy;
pub mod links;
pub mod markdown;
pub mod motion;
pub mod nav;
pub mod order;
pub mod pages;
pub mod presence;
pub mod project;
pub mod scorecard;
pub mod spell;
pub mod status;
pub mod surface;
pub mod sync;
pub mod terminal;
pub mod toolname;
