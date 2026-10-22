---
name: accessibility-check
description: Check keyboard use, focus, names, contrast and motion before an interface is called done.
kind: technique
roles: designer,frontend,qa
---

# Accessibility check

- **Keyboard.** Everything that can be clicked can be reached and used with Tab, Enter, Space and Escape, in an order that follows the visual order.
- **Focus.** The focused element is always visible. Opening a dialog moves focus into it; closing it returns focus to what opened it.
- **Names.** Every control has an accessible name: a visible label, or an aria-label for icon buttons. Inputs are tied to their labels. Images that carry meaning have text; decorative ones are hidden from assistive technology.
- **Structure.** Headings follow a sensible order; lists are lists; buttons that act are buttons and links that navigate are links.
- **Contrast.** Text and the borders of controls are readable against their background, including in the disabled and placeholder states.
- **Colour.** Nothing is communicated by colour alone; an error also says so in words.
- **Motion.** Animation is short, and respects the reduced-motion setting.
- **Zoom.** The layout survives text enlarged to twice its size.

Report each failure with the element and the fix.
