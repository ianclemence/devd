"""Generate every devd icon asset from one canonical logo.

The artwork lives in `assets/devd-logo.png` and nothing else is a source of
truth. This script produces, from that one file:

  * `crates/devd-desktop/icons/*.png`  - the sizes Tauri and Windows want
  * `crates/devd-desktop/icons/icon.ico` - one file with every size inside
  * `crates/devd-server/ui/favicon.svg` - the same artwork as a vector wrapper
  * the inline brand mark in `crates/devd-server/ui/index.html`

Run it after changing the logo:

    python scripts/make-icons.py

It needs Pillow (`pip install pillow`). It is deterministic: the same source
always produces byte-identical assets, so a rebuild never churns the diff.
"""

from __future__ import annotations

import base64
import io
import os
import struct
import sys

try:
    from PIL import Image
except ImportError:  # pragma: no cover - a missing build tool
    sys.exit("Pillow is required: pip install pillow")

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE = os.path.join(ROOT, "assets", "devd-logo.png")
ICON_DIR = os.path.join(ROOT, "crates", "devd-desktop", "icons")
FAVICON = os.path.join(ROOT, "crates", "devd-server", "ui", "favicon.svg")
INDEX = os.path.join(ROOT, "crates", "devd-server", "ui", "index.html")

# The sizes Windows and Tauri ask for.
SIZES = [16, 24, 32, 48, 64, 128, 256, 512]
# The files that are written next to the artwork.
FILES = {
    "32x32.png": 32,
    "64x64.png": 64,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 512,
}
# The sizes inside the .ico, smallest first.
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]


def load_source() -> Image.Image:
    if not os.path.isfile(SOURCE):
        sys.exit(f"the logo is missing: {SOURCE}")
    image = Image.open(SOURCE)
    if image.mode not in ("RGB", "RGBA"):
        image = image.convert("RGBA")
    return image


def has_transparency(image: Image.Image) -> bool:
    """Whether the artwork actually uses an alpha channel."""
    if image.mode != "RGBA":
        return False
    return image.getchannel("A").getextrema()[0] < 255


def render(source: Image.Image, size: int, keep_alpha: bool) -> Image.Image:
    """One size of the artwork, scaled without cropping or distorting it."""
    if source.width == source.height:
        scaled = source.resize((size, size), Image.LANCZOS)
    else:
        # A non-square logo is letterboxed on a transparent square, so the
        # proportions of the mark are preserved rather than stretched.
        side = min(source.width, source.height)
        left = (source.width - side) // 2
        top = (source.height - side) // 2
        square = source.crop((left, top, left + side, top + side))
        canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        canvas.paste(square.resize((size, size), Image.LANCZOS), (0, 0))
        scaled = canvas
    return scaled if keep_alpha else scaled.convert("RGB")


def write_ico(source: Image.Image, keep_alpha: bool) -> None:
    """A .ico holding every size Windows asks for.

    The entries are scaled from the artwork itself, not from a larger file that
    was already an upscale of it, so the 16 and 24 pixel entries stay crisp.
    """
    master = render(source, max(ICO_SIZES), keep_alpha)
    master.save(
        os.path.join(ICON_DIR, "icon.ico"),
        format="ICO",
        sizes=[(size, size) for size in ICO_SIZES],
    )


def write_favicon(source: Image.Image, keep_alpha: bool) -> None:
    """The favicon: the same artwork, clipped to a rounded square."""
    png = io.BytesIO()
    render(source, 128, keep_alpha).save(png, format="PNG", optimize=True)
    encoded = base64.b64encode(png.getvalue()).decode("ascii")
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">'
        '<clipPath id="c"><rect width="128" height="128" rx="22"/></clipPath>'
        f'<image href="data:image/png;base64,{encoded}" '
        'width="128" height="128" clip-path="url(#c)"/></svg>\n'
    )
    with open(FAVICON, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(svg)


def write_inline_logo(source: Image.Image, keep_alpha: bool) -> None:
    """The brand mark in the web interface, as one inline PNG.

    It is RGB on purpose: the artwork has no transparency to lose, and an
    opaque mark cannot show a transparent frame on a dark surface.
    """
    png = io.BytesIO()
    render(source, 128, keep_alpha).save(png, format="PNG", optimize=True)
    encoded = base64.b64encode(png.getvalue()).decode("ascii")
    with open(INDEX, "r", encoding="utf-8") as handle:
        html = handle.read()
    start = html.find("data:image/png;base64,")
    if start == -1:
        sys.exit("index.html has no inline logo to replace")
    end = html.find('"', start)
    html = html[:start] + "data:image/png;base64," + encoded + html[end:]
    with open(INDEX, "w", encoding="utf-8", newline="") as handle:
        handle.write(html)


def main() -> None:
    source = load_source()
    keep_alpha = has_transparency(source)
    os.makedirs(ICON_DIR, exist_ok=True)
    for name, size in FILES.items():
        render(source, size, keep_alpha).save(os.path.join(ICON_DIR, name), format="PNG", optimize=True)
        print(f"wrote crates/devd-desktop/icons/{name} ({size}x{size})")
    write_ico(source, keep_alpha)
    print("wrote crates/devd-desktop/icons/icon.ico (" + ", ".join(str(s) for s in ICO_SIZES) + ")")
    write_favicon(source, keep_alpha)
    print("wrote crates/devd-server/ui/favicon.svg")
    write_inline_logo(source, keep_alpha)
    print("wrote the inline brand mark in crates/devd-server/ui/index.html")


if __name__ == "__main__":
    main()
