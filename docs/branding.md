# Branding: one logo, everywhere

The devd mark has one source: `assets/devd-logo.png`. Nothing else in the
repository is allowed to be a source of truth for the artwork, because the
surfaces that show it are different sizes and different formats, and hand-made
copies of a logo drift.

## What is generated, and from what

`scripts/make-icons.py` reads the one artwork and writes:

| Asset | Size | Used by |
|---|---|---|
| `crates/devd-desktop/icons/32x32.png` | 32 | the installer, the taskbar at small sizes |
| `crates/devd-desktop/icons/64x64.png` | 64 | the installer |
| `crates/devd-desktop/icons/128x128.png` | 128 | the installer, the desktop bundle |
| `crates/devd-desktop/icons/128x128@2x.png` | 256 | high-density displays |
| `crates/devd-desktop/icons/icon.png` | 512 | the source for the largest sizes |
| `crates/devd-desktop/icons/icon.ico` | 16, 24, 32, 48, 64, 128, 256 | Windows: the taskbar, the title bar, the shortcut |
| `crates/devd-server/ui/favicon.svg` | 128 | the browser tab, the window |
| the inline mark in `crates/devd-server/ui/index.html` | 128 | the sidebar, the window chrome |

Run it after changing the artwork:

```
python scripts/make-icons.py
```

It needs Pillow (`pip install pillow`). It is deterministic: the same source
always produces the same bytes, so a rebuild never churns the diff.

## The rules the generator follows

* **One artwork, scaled.** The mark is never cropped and never stretched. A
  logo that is not square is letterboxed on a transparent square so its
  proportions survive.
* **The same pixels everywhere.** The in-app mark and the 128 pixel desktop
  icon are the same file, byte for byte. A test in
  `crates/devd-server/tests/ui_contract.rs` fails if they ever diverge.
* **Opaque unless the artwork needs otherwise.** The supplied artwork has no
  transparency, so every generated file is an opaque RGB PNG. An opaque mark
  cannot show a transparent frame on a dark surface, which is the failure the
  original logo had.
* **Small sizes are scaled from the artwork, not from an upscale of it.** The
  16 and 24 pixel entries in the `.ico` are the sharpest they can be, which
  is what the taskbar actually shows.

## The terminal

A terminal cannot show a picture, so the terminal interface uses the wordmark:
`devd` in the header, in the same accent colour as the window. The name, the
capitalisation and the colour are the same in the window, the installer, the
CLI and the dashboard.

## Changing the logo

1. Replace `assets/devd-logo.png`. Keep it square; 512 pixels or more is
   enough for every size.
2. Run `python scripts/make-icons.py`.
3. `cargo test -p devd-server --test ui_contract` - the branding test checks
   that the artwork is still square, still opaque and still the same in both
   places.
4. Rebuild and reinstall: `scripts/release-installer.ps1`, then run the
   installer it writes to `dist/`.
