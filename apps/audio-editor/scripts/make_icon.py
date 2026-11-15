"""Generate assets/icon/app.png and assets/icon/app.ico.

Renders the start screen's 20-bar logo (THEME.start_bar_heights, colors
theme::track_color(i / 3)) on a #1A1D22 rounded square. Run from the repo
root: python scripts/make_icon.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

# From src/ui/theme.rs (Theme defaults).
BAR_W = 4.0
BAR_PITCH = 6.0
BAR_HEIGHTS = [8, 16, 26, 34, 22, 30, 38, 24, 14, 28, 36, 20, 30, 18, 10, 24, 32, 16, 8, 12]
TRACK_COLORS = [
    (0x4F, 0xB3, 0xA9), (0x9C, 0x8C, 0xE0), (0xD3, 0xA8, 0x6A), (0xD9, 0x7A, 0x8C),
    (0x7B, 0xC4, 0x7F), (0x5F, 0xA8, 0xD3), (0xC9, 0x8B, 0xC9), (0xB8, 0xB8, 0x6A),
]

BG = (0x1A, 0x1D, 0x22)
SIZE = 256
ICON_SIZES = [16, 24, 32, 48, 64, 256]


def render(size: int) -> Image.Image:
    """Render the logo at `size` x `size` px (one supersampled pass per size)."""
    # Scale factors: the on-screen logo is 40 px tall in a 120 px wide run of
    # bars (20 bars * 6 pitch); use a 2x logo scale so 256 gets 240 px of bars.
    logo_scale = size / 240.0
    scale = 2.0 * logo_scale
    ss = 4  # supersample
    px = size * ss
    img = Image.new("RGBA", (px, px), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # Rounded-square background, ~20% corner radius.
    r = int(px * 0.20)
    d.rounded_rectangle([0, 0, px - 1, px - 1], radius=r, fill=BG + (255,))

    # Bars, centered horizontally and vertically (logo box = pitch run x 40*2).
    bar_w = BAR_W * scale * ss
    pitch = BAR_PITCH * scale * ss
    total_w = (len(BAR_HEIGHTS) - 1) * pitch + bar_w
    x0 = (px - total_w) / 2.0
    logo_h = 40.0 * scale * ss
    cy = px / 2.0
    for i, bh in enumerate(BAR_HEIGHTS):
        h = bh * scale * ss
        rect = [x0 + i * pitch, cy - h / 2.0, x0 + i * pitch + bar_w, cy + h / 2.0]
        col = TRACK_COLORS[i // 3] + (255,)
        d.rounded_rectangle(rect, radius=bar_w / 2.0, fill=col)

    return img.resize((size, size), Image.LANCZOS)


def main() -> None:
    out = Path(__file__).resolve().parent.parent / "assets" / "icon"
    out.mkdir(parents=True, exist_ok=True)

    img = render(SIZE)
    img.save(out / "app.png")

    sizes = [(s, s) for s in ICON_SIZES]
    img.save(out / "app.ico", sizes=sizes)
    print(f"wrote {out / 'app.png'} and {out / 'app.ico'}")


if __name__ == "__main__":
    main()
