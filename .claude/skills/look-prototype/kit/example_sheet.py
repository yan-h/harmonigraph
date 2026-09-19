"""The whole pattern in one file, with a toy effect standing in for a real look.

Copy this next to your own renderer and replace `warp` -- the rest (a two-axis
sweep at ss=1, an ID and the parameters and a measured tag in every label, the
flat-input control, one montaged PNG) is the shape every round wants.
Run: python3 example_sheet.py
"""
import os, sys, time
import numpy as np
import measure, pane, spectro as S
from png import label_cell, sheet, write_png

TMP = os.path.dirname(os.path.abspath(__file__))
W, H = 400, 225                 # one cell, ss=1: what the shader would draw
CELLS = (8.0, 20.0, 50.0)       # px, the size of one warp cell
AMOUNTS = (2.0, 6.0, 16.0)      # px, how far it pulls the light field


def warp(L, cell, amount):
    """Toy effect: sample the light field along a wobbling sine field.

    Per-pixel, no neighbourhood, no global pass -- the shape a port can follow.
    """
    h, w = L.shape
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    k = 2.0 * np.pi / cell
    u = np.sin(k * x + 1.7 * np.sin(0.6 * k * y))
    v = np.cos(k * y + 1.3 * np.sin(0.4 * k * x))
    sx = np.clip(x + amount * u, 0, w - 1).astype(np.int32)
    sy = np.clip(y + amount * v, 0, h - 1).astype(np.int32)
    return L[sy, sx]


def cell_image(ident, cell, amount, spacing, flat=None):
    t = time.time()
    L, _ = pane.pane_light(W, H, flat=flat)
    out = S.palette(warp(L, cell, amount))
    if flat is None:
        note = measure.tag(*measure.legibility(S.palette(L), out, spacing))
    else:
        # The legibility measure is meaningless against a constant field, so the
        # flat control reports what the effect INVENTED out of nothing instead.
        # Luminance, not the raw RGB: a constant COLOUR has a spread across its
        # three channels, and that spread is not structure on the screen.
        note = "FLAT IN %.2f  INVENTS STD %.3f" % (flat, float(measure.lum(out).std()))
    img = S.to_srgb8(out)
    label_cell(img, ["%s  cell %g  amt %g" % (ident, cell, amount), note], scale=2)
    print("   %-10s %.2fs  %s" % (ident, time.time() - t, note))
    sys.stdout.flush()
    return img


if __name__ == "__main__":
    t0 = time.time()
    L, _ = pane.pane_light(W, H)
    spacing = measure.line_spacing(L)[0]
    print("pane %dx%d, harmonic spacing %d px, take %s"
          % (W, H, spacing, os.path.basename(pane.WAV)))

    cells = []
    for r, c in enumerate(CELLS):
        for k, a in enumerate(AMOUNTS):
            cells.append(cell_image("%s%d" % ("ABC"[r], k + 1), c, a, spacing))

    base = S.to_srgb8(S.palette(L))
    label_cell(base, ["BASE  no effect", "spacing %d px" % spacing], scale=2)
    cells.append(base)
    cells.append(cell_image("D1", CELLS[1], AMOUNTS[1], spacing, flat=0.34))

    # A title wider than the sheet is CLIPPED, not wrapped: at tscale 3 a
    # character is 18 px, so keep it under (sheet width / 18) characters. A
    # character the 5x7 font lacks (';' is one) silently draws as '?'.
    p = write_png(os.path.join(TMP, "example_sheet.png"), sheet(
        cells, 3,
        title="EXAMPLE  cell size (rows) x amount (cols), ss=1 - base + flat"))
    print("=> %s  (%.1fs total)" % (p, time.time() - t0))
