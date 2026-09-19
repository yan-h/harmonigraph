"""Harmonic line spacing in the crop, and a legibility number per render.

Answers "do the harmonics still read through this effect" as a number you put
in the cell's label, so a sheet can be argued from rather than squinted at.
DENSE_X and BAND_Y are fractions of the crop, picked for the default excerpt:
re-check them against a picture of the base before trusting them on another.
"""
import numpy as np
import spectro as S

LUMW = np.array([0.2126, 0.7152, 0.0722], np.float32)


def lum(rgb):
    return (rgb * LUMW).sum(-1)


def _gauss1(a, sigma):
    if sigma <= 0:
        return a.copy()
    r = max(1, int(3 * sigma))
    k = np.exp(-0.5 * (np.arange(-r, r + 1) / sigma) ** 2).astype(np.float32)
    k /= k.sum()
    return np.convolve(np.pad(a, (r, r), mode="edge"), k, mode="same")[r:-r]


def hp(a, sigma):
    return a - _gauss1(a, sigma)


def corr(a, b):
    a = a - a.mean(); b = b - b.mean()
    d = np.sqrt((a * a).sum() * (b * b).sum())
    return float((a * b).sum() / d) if d > 1e-12 else 0.0


# The dense passage and the harmonic band, as fractions of the crop.
DENSE_X = (0.02, 0.50)
BAND_Y = (0.10, 0.80)


def ridges(baseL):
    """Rows of the harmonic ridges in the dense passage.

    On a LOG-frequency axis a harmonic series has no single period -- the gaps
    shrink as the partial number rises -- so this counts the ridges rather than
    autocorrelating for one spacing.
    """
    H, W = baseL.shape
    x0, x1 = int(DENSE_X[0] * W), int(DENSE_X[1] * W)
    prof = baseL[:, x0:x1].mean(1).astype(np.float32)
    d = hp(prof, 0.035 * H)
    thr = 0.35 * d.std()
    pk = [r for r in range(1, H - 1)
          if d[r] > d[r - 1] and d[r] >= d[r + 1] and d[r] > thr]
    return np.array(pk), prof, d


def line_spacing(baseL):
    """(median, min, max) rows between adjacent ridges, and the ridge count."""
    pk, _, _ = ridges(baseL)
    if len(pk) < 3:
        return 0, 0, 0, len(pk)
    g = np.diff(pk)
    g = g[g <= 0.25 * baseL.shape[0]]
    return int(np.median(g)), int(g.min()), int(np.percentile(g, 90)), len(pk)


def legibility(base_rgb, out_rgb, spacing):
    """(pitch correlation, pitch contrast ratio, time correlation).

    pitch: row-mean profile over the dense columns, high-passed at 1.5 line
    spacings -- 'did the harmonic lines survive'.
    time: column-mean profile over the harmonic band, high-passed at 10 px --
    'did the onsets survive'.
    """
    H, W, _ = base_rgb.shape
    bl, ol = lum(base_rgb), lum(out_rgb)
    x0, x1 = int(DENSE_X[0] * W), int(DENSE_X[1] * W)
    y0, y1 = int(BAND_Y[0] * H), int(BAND_Y[1] * H)
    sp = 1.5 * spacing
    bp, op = hp(bl[:, x0:x1].mean(1), sp), hp(ol[:, x0:x1].mean(1), sp)
    rP = corr(bp, op)
    cP = float(op.std() / max(bp.std(), 1e-9))
    # The time axis of this crop is 93% slow decay -- high-passing it at any
    # scale leaves noise, so TIME is the ENVELOPE correlation (loud passage vs
    # decay), not an onset measure. Stated plainly rather than dressed up.
    bt, ot = bl[y0:y1].mean(0), ol[y0:y1].mean(0)
    rT = corr(bt, ot)
    return rP, cP, rT


def tag(rP, cP, rT):
    return "PITCH r%.2f c%.2f  TIME %.2f" % (rP, cP, rT)


if __name__ == "__main__":
    import pane
    for (w, h) in ((960, 540), (1920, 1080)):
        L, _ = pane.pane_light(w, h)
        med, lo, hi, n = line_spacing(L)
        pk, _, _ = ridges(L)
        print("%dx%d: %d ridges, spacing median %d px (tightest %d, 90th pct %d)"
              % (w, h, n, med, lo, hi))
        print("   ridge rows:", " ".join(str(int(r)) for r in pk[:40]))
        base = S.palette(L)
        print("   self-legibility", tag(*legibility(base, base, med)))
