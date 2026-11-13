//! Transitions, the way Tailwind names them: a duration and an easing, set once, used everywhere.
//!
//! `transition duration-150 ease-out` is a class on an element there; here it is a function around one. The curves are
//! Tailwind's own (`ease-in-out` is `cubic-bezier(0.4, 0, 0.2, 1)`, `ease-out` is `cubic-bezier(0, 0, 0.2, 1)`), evaluated by `devd_app_core::motion`, which has the arithmetic and its tests.
//!
//! What a transition is for: something that appears, appears from where it came, quickly, and settles; it never delays
//! what the person asked for. An element enters the moment it is first drawn and is not touched again, so nothing
//! animates on every frame and nothing keeps the window redrawing once it has arrived. The window has no CSS `:hover`
//! to transition, so a hover is a change of colour at once, which is what a response to a pointer should be.

use std::time::Duration;

use devd_app_core::motion::Ease;
use gpui::{Animation, AnimationElement, AnimationExt, ElementId, IntoElement, Styled, px};

/// Tailwind's `ease-in-out`.
pub fn ease_in_out(t: f32) -> f32 {
    Ease::Bezier(0.4, 0.0, 0.2, 1.0).at(t)
}

/// Tailwind's `ease-out`: quick to start, long to settle. For what enters.
pub fn ease_out(t: f32) -> f32 {
    Ease::Bezier(0.0, 0.0, 0.2, 1.0).at(t)
}

/// Fade in while rising `rise` pixels into place: `transition duration-150 ease-out`, from `opacity-0 translate-y-1`.
///
/// `id` must differ whenever the thing is a different thing (a screen's path, a popover's name), so a new one enters and
/// the same one is left alone.
pub fn rise<E>(
    el: E,
    id: impl Into<ElementId>,
    rise: f32,
    duration: Duration,
) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(
        id,
        Animation::new(duration).with_easing(ease_out),
        move |el, t| el.relative().top(px((1.0 - t) * rise)).opacity(t),
    )
}

/// Fade in, in place: `transition-opacity duration-150 ease-in-out`.
pub fn fade<E>(el: E, id: impl Into<ElementId>, duration: Duration) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(
        id,
        Animation::new(duration).with_easing(ease_in_out),
        |el, t| el.opacity(t),
    )
}

/// Slide in from the right edge by `from` pixels while fading in: a panel opening.
pub fn slide_in<E>(
    el: E,
    id: impl Into<ElementId>,
    from: f32,
    duration: Duration,
) -> AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    el.with_animation(
        id,
        Animation::new(duration).with_easing(ease_out),
        move |el, t| el.relative().left(px((1.0 - t) * from)).opacity(t),
    )
}

/// The durations, from `devd_app_core::motion::duration`: 140 ms for a popover or a screen, 200 ms for a panel or a dialog.
pub use devd_app_core::motion::duration::{BASE, FAST};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curves_start_at_nothing_end_at_everything_and_never_go_backwards() {
        for curve in [ease_in_out as fn(f32) -> f32, ease_out] {
            assert!(curve(0.0).abs() < 0.001 && (curve(1.0) - 1.0).abs() < 0.001);
            let mut last = 0.0;
            for step in 0..=100 {
                let y = curve(step as f32 / 100.0);
                assert!(
                    y >= last - 0.0001,
                    "an easing that goes backwards would make a thing flicker"
                );
                last = y;
            }
        }
    }

    #[test]
    fn ease_out_is_ahead_of_a_straight_line_early() {
        assert!(ease_out(0.25) > 0.25);
        assert!(
            ease_in_out(0.1) < 0.1 && ease_in_out(0.9) > 0.9,
            "in-out is slow at both ends"
        );
    }
}
