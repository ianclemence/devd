//! A line of text that ends in an ellipsis instead of being cut mid-letter.
//!
//! gpui has a `truncate` for this, and for text that does not wrap it does
//! nothing useful inside a flex layout: it decides how much to keep from the
//! first size it is asked about, before the layout has said how wide the line
//! really is, and never decides again. The line is then simply clipped by its
//! container, half a letter and no sign that anything is missing.
//!
//! This element waits for the width it was actually given. It measures at paint
//! time, and if the text is wider than its box it keeps what fits and ends in
//! "…", or, for a path, keeps the end and starts with "…", because the end is
//! the part that tells two folders apart.
//!
//! It draws one line in the text style of its parent: colour, size, family and
//! weight are set on the element around it, as for any other text.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, SharedString, Style, TextRun, Window, px, size,
};

/// Where the missing text is taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cut {
    End,
    Start,
}

/// One line of text, shortened to fit.
pub struct Clip {
    text: SharedString,
    cut: Cut,
}

impl Clip {
    /// A line that loses its end when it is too long.
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            cut: Cut::End,
        }
    }

    /// A line that loses its beginning, for a path or an identifier whose end matters.
    pub fn tail(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            cut: Cut::Start,
        }
    }
}

impl IntoElement for Clip {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The text shortened to `room` pixels, as the pieces to draw. Pure, so the choice
/// of where to cut is the same one the tests make.
fn fit(
    text: &str,
    cut: Cut,
    room: Pixels,
    width_of: &dyn Fn(&str) -> Pixels,
    index_for: &dyn Fn(&str, Pixels) -> usize,
) -> String {
    if width_of(text) <= room {
        return text.to_string();
    }
    let ellipsis = width_of("…");
    let left = (room - ellipsis).max(px(0.0));
    match cut {
        Cut::End => {
            let end = floor_boundary(text, index_for(text, left));
            format!("{}…", text[..end].trim_end())
        }
        Cut::Start => {
            let from = ceil_boundary(text, index_for(text, width_of(text) - left));
            format!("…{}", text[from..].trim_start())
        }
    }
}

fn floor_boundary(text: &str, mut i: usize) -> usize {
    i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(text: &str, mut i: usize) -> usize {
    i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

impl Element for Clip {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let _ = cx;
        let (text, style) = (self.text.clone(), window.text_style());
        let font_size = style.font_size.to_pixels(window.rem_size());
        let height = window.line_height();
        // It asks for the width of the whole text, and may be given less: it can shrink to nothing.
        let mut layout = Style::default();
        layout.min_size.width = px(0.0).into();
        let id = window.request_measured_layout(layout, move |known, _available, window, _cx| {
            let run = TextRun {
                len: text.len(),
                font: style.font(),
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let natural = window
                .text_system()
                .shape_line(text.clone(), font_size, &[run], None)
                .width;
            size(
                known.width.unwrap_or(natural),
                known.height.unwrap_or(height),
            )
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let shape = |window: &Window, text: &str| {
            let run = TextRun {
                len: text.len(),
                font: style.font(),
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window.text_system().shape_line(
                SharedString::from(text.to_string()),
                font_size,
                &[run],
                None,
            )
        };
        let shown = fit(
            &self.text,
            self.cut,
            bounds.size.width,
            &|t| shape(window, t).width,
            &|t, x| shape(window, t).closest_index_for_x(x),
        );
        let line = shape(window, &shown);
        let _ = line.paint(bounds.origin, window.line_height(), window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // One pixel a character: the choice of where to cut does not depend on a font.
    fn width_of(t: &str) -> Pixels {
        px(t.chars().count() as f32)
    }
    fn index_for(t: &str, x: Pixels) -> usize {
        t.char_indices()
            .nth(f32::from(x).max(0.0) as usize)
            .map(|(i, _)| i)
            .unwrap_or(t.len())
    }

    #[test]
    fn text_that_fits_is_left_alone() {
        assert_eq!(
            fit(
                "Design the dashboard",
                Cut::End,
                px(40.0),
                &width_of,
                &index_for
            ),
            "Design the dashboard"
        );
    }

    #[test]
    fn a_long_title_ends_in_an_ellipsis_within_its_room() {
        let shown = fit(
            "Implement the Mission Control operations console",
            Cut::End,
            px(20.0),
            &width_of,
            &index_for,
        );
        assert_eq!(shown, "Implement the Missi…");
        assert!(shown.chars().count() <= 20);
    }

    #[test]
    fn a_long_path_keeps_its_end() {
        let shown = fit(
            r"C:\Users\User\AppData\Local\Temp\scratchpad\home",
            Cut::Start,
            px(20.0),
            &width_of,
            &index_for,
        );
        assert!(
            shown.starts_with('…') && shown.ends_with(r"scratchpad\home"),
            "{shown}"
        );
        assert!(shown.chars().count() <= 20, "{shown}");
    }

    #[test]
    fn a_cut_never_lands_inside_a_letter() {
        let shown = fit(
            "naïve façade café ☕ done",
            Cut::End,
            px(9.0),
            &width_of,
            &index_for,
        );
        assert!(shown.ends_with('…') && shown.is_char_boundary(shown.len() - '…'.len_utf8()));
        let tail = fit(
            "naïve façade café ☕ done",
            Cut::Start,
            px(9.0),
            &width_of,
            &index_for,
        );
        assert!(tail.starts_with('…'));
    }

    #[test]
    fn no_room_at_all_is_just_an_ellipsis_not_a_panic() {
        assert_eq!(
            fit("anything", Cut::End, px(0.0), &width_of, &index_for),
            "…"
        );
        assert_eq!(
            fit("anything", Cut::Start, px(0.0), &width_of, &index_for),
            "…"
        );
    }
}
