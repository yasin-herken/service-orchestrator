#!/usr/bin/env python3
"""Generate the Service Orchestrator application icon source.

The icon is an **original placeholder**, not final branding (Task 031 §8). It
draws a macOS-style rounded square with an "orchestration" motif — one hub
service connected to three dependent services — and writes a single
1024x1024 PNG. `npm run tauri icon` (or `npx tauri icon`) turns that source
into the full macOS/Windows/Linux icon set (`icon.icns`, the sized PNGs, and
`icon.ico`).

The generated PNG is committed, so a normal build never runs this script; it
exists so the placeholder can be regenerated reproducibly if the motif changes.

Usage:
    python3 apps/desktop/scripts/generate-icon.py [output.png]
"""

from __future__ import annotations

import math
import sys
from pathlib import Path

from PIL import Image, ImageDraw

# Apple's macOS icon grid: an 824x824 rounded square centred on a 1024 canvas,
# leaving the platform-standard transparent margin.
CANVAS = 1024
SQUIRCLE = 824
MARGIN = (CANVAS - SQUIRCLE) // 2
RADIUS = 185
SS = 4  # supersampling factor for smooth edges.

BACKGROUND_TOP = (30, 43, 92, 255)
BACKGROUND_BOTTOM = (45, 108, 223, 255)
ACCENT = (255, 255, 255, 255)
ACCENT_SOFT = (255, 255, 255, 150)


def rounded_mask(size: int, radius: int, supersample: int) -> Image.Image:
    """Returns an anti-aliased rounded-rectangle mask."""
    big = Image.new("L", (size * supersample, size * supersample), 0)
    draw = ImageDraw.Draw(big)
    draw.rounded_rectangle(
        (0, 0, size * supersample - 1, size * supersample - 1),
        radius=radius * supersample,
        fill=255,
    )
    return big.resize((size, size), Image.LANCZOS)


def vertical_gradient(size: int, top: tuple, bottom: tuple) -> Image.Image:
    """Returns a vertical linear gradient image."""
    gradient = Image.new("RGBA", (1, size))
    for y in range(size):
        t = y / max(size - 1, 1)
        gradient.putpixel(
            (0, y),
            tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(4)),
        )
    return gradient.resize((size, size))


def node(draw: ImageDraw.ImageDraw, centre: tuple[float, float], radius: float, fill) -> None:
    """Draws a filled circle with a soft anti-aliased outline."""
    x, y = centre
    draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=fill)


def build() -> Image.Image:
    canvas = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))

    # Rounded-square background with a vertical gradient.
    background = vertical_gradient(SQUIRCLE, BACKGROUND_TOP, BACKGROUND_BOTTOM)
    mask = rounded_mask(SQUIRCLE, RADIUS, SS)
    canvas.paste(background, (MARGIN, MARGIN), mask)

    # Draw the motif at supersampled scale, then downscale for smooth curves.
    layer = Image.new("RGBA", (SQUIRCLE * SS, SQUIRCLE * SS), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    scale = SS

    def point(fx: float, fy: float) -> tuple[float, float]:
        return (fx * SQUIRCLE * scale, fy * SQUIRCLE * scale)

    hub = point(0.5, 0.30)
    children = [point(0.26, 0.72), point(0.5, 0.78), point(0.74, 0.72)]

    # Connectors from the hub to each dependent node.
    for child in children:
        draw.line((hub, child), fill=ACCENT_SOFT, width=int(14 * scale))

    # Dependent nodes, then the hub on top.
    for child in children:
        node(draw, child, 62 * scale, ACCENT)
    node(draw, hub, 96 * scale, ACCENT)

    # An inner ring around the hub, so the hub reads as "the orchestrator".
    x, y = hub
    ring = 140 * scale
    draw.ellipse(
        (x - ring, y - ring, x + ring, y + ring),
        outline=ACCENT_SOFT,
        width=int(12 * scale),
    )

    layer = layer.resize((SQUIRCLE, SQUIRCLE), Image.LANCZOS)
    # Mask the motif to the squircle so nothing bleeds past the rounded corners.
    canvas.paste(layer, (MARGIN, MARGIN), Image.composite(layer.getchannel("A"), Image.new("L", layer.size, 0), mask))

    return canvas


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).parent.parent / "icon-source.png"
    image = build()
    image.save(out)
    print(f"wrote {out} ({image.width}x{image.height})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
