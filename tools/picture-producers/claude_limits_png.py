#!/usr/bin/env python3
"""Draw the Claude usage panel as a 448x368 PNG and push it to a Deskmate image source.

This is what a picture-card producer looks like. It reads whatever data it likes,
draws the finished face itself, and POSTs the bytes. There is no manifest, no
expression language and no scene -- which is the whole point of the card kind.

    claude_limits_png.py --feed URL --push URL [--out FILE] [--dry-run]
"""

import argparse
import json
import sys
import urllib.error
import urllib.request

from PIL import Image, ImageDraw, ImageFont

# Fixed canvas. Anything else is refused at ingest with a 422.
WIDTH, HEIGHT = 448, 368

# Cloudflare fronts both the feed and the push endpoint, and blocks urllib's
# default User-Agent outright. One name, used for both requests.
PRODUCER_USER_AGENT = "deskmate-picture-producer/1"

# DESIGN.md stages on true black in both colour schemes, and an AMOLED pixel
# costs nothing to leave off.
BLACK = (0, 0, 0)
WHITE = (255, 255, 255)
GREY = (138, 138, 142)
TRACK = (28, 28, 30)

# Bars are neutral so usage level is carried by length rather than an extra
# status colour that would compete with the percentage.
BAR = (255, 255, 255)

GRID = 8

FONT_CANDIDATES = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans{bold}.ttf",
    "/System/Library/Fonts/Supplemental/DejaVuSans{bold}.ttf",
    "/Library/Fonts/Arial{bold_mac}.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
]


def load_font(size, bold=False):
    """First font that exists, so this runs on the VM and on a Mac unchanged."""
    for template in FONT_CANDIDATES:
        path = template.format(
            bold="-Bold" if bold else "",
            bold_mac=" Bold" if bold else "",
        )
        try:
            return ImageFont.truetype(path, size)
        except OSError:
            continue
    # Never silently fall back to a 6px bitmap face: a panel nobody can read is
    # worse than a producer that says why it failed.
    raise SystemExit(
        "no usable TrueType font found; tried:\n  " + "\n  ".join(FONT_CANDIDATES)
    )


def fetch(url):
    # Cloudflare fronts the feed and answers urllib's default User-Agent with a
    # 403, so identify ourselves. This is not a workaround for a control; the
    # feed is deliberately public because the egress guard forbids the server
    # fetching a private address.
    request = urllib.request.Request(url, headers={"User-Agent": PRODUCER_USER_AGENT})
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.loads(response.read().decode("utf-8"))


def draw_panel(payload):
    image = Image.new("RGB", (WIDTH, HEIGHT), BLACK)
    draw = ImageDraw.Draw(image)

    title = load_font(26, bold=True)
    label = load_font(19, bold=True)
    number = load_font(46, bold=True)
    caption = load_font(15)

    margin = 3 * GRID
    draw.text((margin, 2 * GRID), "CLAUDE", font=title, fill=WHITE)
    plan = str(payload.get("plan", "")).strip()
    if plan:
        right = draw.textlength(plan, font=caption)
        draw.text(
            (WIDTH - margin - right, 2 * GRID + 10), plan, font=caption, fill=GREY
        )

    windows = payload.get("windows") or []
    # Two stacked full-width rows make each percentage point about 4px of bar,
    # keeping small usage changes visible on the fixed-size panel.
    top = 9 * GRID
    row_height = 15 * GRID
    for index, window in enumerate(windows[:2]):
        origin = top + index * row_height
        name = str(window.get("name", "?"))
        used = window.get("used_pct")
        used = 0 if used is None else max(0, min(100, int(used)))

        draw.text((margin, origin), name.upper(), font=label, fill=WHITE)
        percent = f"{used}%"
        right = draw.textlength(percent, font=number)
        draw.text((WIDTH - margin - right, origin - 14), percent, font=number, fill=WHITE)

        bar_top = origin + 5 * GRID
        bar_width = WIDTH - 2 * margin
        draw.rounded_rectangle(
            [margin, bar_top, margin + bar_width, bar_top + 14], radius=7, fill=TRACK
        )
        if used > 0:
            filled = max(14, int(bar_width * used / 100))
            draw.rounded_rectangle(
                [margin, bar_top, margin + filled, bar_top + 14], radius=7, fill=BAR
            )

        resets = str(window.get("resets_at_label", "")).strip()
        if resets:
            draw.text(
                (margin, bar_top + 22), f"RESETS {resets}", font=caption, fill=GREY
            )

    # The bottom strip is kept deliberately quiet: a stale footer overlays the
    # picture, because a picture is full-bleed and reserves no strip for it.
    return image


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--feed", required=True)
    parser.add_argument("--push")
    parser.add_argument("--out")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    payload = fetch(args.feed)
    image = draw_panel(payload)

    if args.out:
        image.save(args.out, format="PNG")
        print(f"wrote {args.out}")

    if args.dry_run or not args.push:
        return 0

    import io

    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    body = buffer.getvalue()

    request = urllib.request.Request(
        args.push,
        data=body,
        method="POST",
        headers={
            "Content-Type": "image/png",
            # Same reason the feed fetch sets one: Cloudflare fronts this host
            # and answers urllib's default User-Agent with a 403 (error 1010).
            # curl gets through on its own UA, which is why a hand-run push
            # worked while the timer's first run did not.
            "User-Agent": PRODUCER_USER_AGENT,
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            print(f"pushed {len(body)} bytes -> {response.status}")
    except urllib.error.HTTPError as error:
        # The server writes these for the producer: one line, no internals.
        print(
            f"push refused {error.code}: "
            f'{error.read().decode("utf-8", errors="replace")}',
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
