"""Render the Cryocode app icon (a frosted `{ }` on a deep-ice squircle) to an .icns file."""
import os
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw, ImageFilter, ImageFont

SIZE = 1024


def squircle_mask(size, radius):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    return m


def render():
    pad = 100  # macOS icon grid: artwork inset inside the 1024 canvas
    inner = SIZE - 2 * pad
    bg = Image.new("RGBA", (inner, inner))
    px = bg.load()
    top, bottom = (0x1E, 0x2A, 0x3D), (0x0E, 0x13, 0x1C)
    for y in range(inner):
        t = y / inner
        c = tuple(int(top[i] * (1 - t) + bottom[i] * t) for i in range(3))
        for x in range(inner):
            px[x, y] = c + (255,)
    # Soft icy glow in the upper left.
    glow = Image.new("RGBA", (inner, inner), (0, 0, 0, 0))
    ImageDraw.Draw(glow).ellipse([-200, -260, inner * 0.8, inner * 0.55], fill=(124, 196, 248, 70))
    glow = glow.filter(ImageFilter.GaussianBlur(120))
    bg = Image.alpha_composite(bg, glow)

    font = None
    for path in ["/System/Library/Fonts/SFNSMono.ttf", "/System/Library/Fonts/Menlo.ttc"]:
        try:
            font = ImageFont.truetype(path, 330)
            break
        except OSError:
            pass
    text = "{ }"
    d = ImageDraw.Draw(bg)
    box = d.textbbox((0, 0), text, font=font)
    w, h = box[2] - box[0], box[3] - box[1]
    pos = ((inner - w) / 2 - box[0], (inner - h) / 2 - box[1] - 20)
    shadow = Image.new("RGBA", (inner, inner), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).text((pos[0], pos[1] + 18), text, font=font, fill=(0, 0, 0, 140))
    bg = Image.alpha_composite(bg, shadow.filter(ImageFilter.GaussianBlur(18)))
    fg = Image.new("RGBA", (inner, inner), (0, 0, 0, 0))
    ImageDraw.Draw(fg).text(pos, text, font=font, fill=(186, 228, 255, 255))
    bg = Image.alpha_composite(bg, fg)
    # A small accent dot: the cursor.
    d = ImageDraw.Draw(bg)
    cx, cy, r = inner / 2, inner * 0.74, 22
    d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(124, 196, 248, 255))

    bg.putalpha(squircle_mask(inner, 185))
    canvas = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    canvas.paste(bg, (pad, pad), bg)
    return canvas


def main(out):
    img = render()
    with tempfile.TemporaryDirectory() as tmp:
        iconset = os.path.join(tmp, "Cryocode.iconset")
        os.makedirs(iconset)
        for s in [16, 32, 128, 256, 512]:
            img.resize((s, s), Image.LANCZOS).save(os.path.join(iconset, f"icon_{s}x{s}.png"))
            img.resize((s * 2, s * 2), Image.LANCZOS).save(os.path.join(iconset, f"icon_{s}x{s}@2x.png"))
        subprocess.check_call(["iconutil", "-c", "icns", iconset, "-o", out])
    img.resize((256, 256), Image.LANCZOS).save(out.replace(".icns", ".png"))


if __name__ == "__main__":
    main(sys.argv[1])
