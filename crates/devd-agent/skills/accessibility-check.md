---
name: accessibility-check
description: Check keyboard, focus, names, contrast and motion before an interface is done
kind: review
roles: designer,frontend,qa
---

# Accessibility check

Checks that an interface can be used without a mouse, a good eye or a fast hand. It reports failures with the fix; it does not redesign the screen.

## When to use

- Before calling any screen, dialog or form done.
- When someone says "check accessibility", "can I use this with the keyboard", or "audit the UI".

## Procedure

1. **Keyboard.** Tab through the whole screen with `browser.key`. Everything clickable is reachable and usable with Tab, Enter, Space and Escape, in an order that follows the visual order.
2. **Focus.** The focused element is always visible. Opening a dialog moves focus into it; closing it returns focus to what opened it.
3. **Names.** Every control has an accessible name: a visible label, or an aria-label on an icon button. Inputs are tied to their labels. Images that carry meaning have text; decorative ones are hidden from assistive technology. `browser.snapshot` shows the names a screen reader would hear.
4. **Structure.** Headings follow an order; lists are lists; things that act are buttons and things that navigate are links.
5. **Contrast.** Text and control borders are readable against their background, including disabled and placeholder states.
6. **Colour.** Nothing is said by colour alone; an error also says so in words.
7. **Motion.** Animation is short and respects the reduced-motion setting.
8. **Zoom.** The layout survives text at twice the size.

## Pitfalls

- Checking in the code and not on the page. A label that exists in the template can still be missing from the rendered tree.
- Fixing contrast on the default state and forgetting hover, focus, disabled and dark mode.
- Treating an automated checker's clean result as the whole check. It cannot judge order, wording or focus.

## Verification

Each failure is listed with the element and the fix, and each fix is re-checked in the browser. Say which of the eight you did not test.
