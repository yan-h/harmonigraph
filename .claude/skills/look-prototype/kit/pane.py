"""Build the pane-sized 'light' field (0..1) for a chosen excerpt, and the base image.

This is what a prototype effect draws over: one real excerpt of one real take,
resampled to whatever pane size you ask for and cached beside this file.
The take and the window are the HARMONIGRAPH_TAKE* environment variables.
"""
import os
import numpy as np
import spectro as S

TMP = os.path.dirname(os.path.abspath(__file__))
WAV = S.DEFAULT_TAKE
# dense passage running into the sparse tail
START = float(os.environ.get("HARMONIGRAPH_TAKE_START", 72.0))
DUR = float(os.environ.get("HARMONIGRAPH_TAKE_DUR", 20.0))

_cache = {}


def _base_db():
    if "db" not in _cache:
        f = os.path.join(TMP, S.cache_name("cache_db", WAV, START, DUR, ".npy"))
        if os.path.exists(f):
            _cache["db"] = np.load(f)
        else:
            x, sr = S.decode(WAV, START, DUR)
            db = S.stft_log(x, sr, n_fft=2048, hop=128, fmin=70.0, fmax=9000.0, rows=540)
            np.save(f, db)
            _cache["db"] = db
    return _cache["db"]


def _resample(a, H, W):
    """bilinear resample a (h,w) -> (H,W)"""
    h, w = a.shape
    yi = (np.arange(H) + 0.5) * h / H - 0.5
    xi = (np.arange(W) + 0.5) * w / W - 0.5
    y0 = np.clip(np.floor(yi).astype(int), 0, h - 1)
    y1 = np.clip(y0 + 1, 0, h - 1)
    x0 = np.clip(np.floor(xi).astype(int), 0, w - 1)
    x1 = np.clip(x0 + 1, 0, w - 1)
    ty = np.clip(yi - y0, 0, 1)[:, None]
    tx = np.clip(xi - x0, 0, 1)[None, :]
    A = a[np.ix_(y0, x0)] * (1 - ty) * (1 - tx)
    A += a[np.ix_(y1, x0)] * ty * (1 - tx)
    A += a[np.ix_(y0, x1)] * (1 - ty) * tx
    A += a[np.ix_(y1, x1)] * ty * tx
    return A.astype(np.float32)


def pane_light(W, H, flat=None):
    """Return (L, Lw): the display level 0..1 and a wide-blurred copy, both (H,W).
    Row 0 is the TOP of the pane = high pitch."""
    if flat is not None:
        L = np.full((H, W), float(flat), np.float32)
        return L, L.copy()
    key = ("L", W, H)
    if key not in _cache:
        db = _base_db()
        # light temporal smoothing, like the analyser's own averaging
        k = np.hanning(9).astype(np.float32); k /= k.sum()
        db = np.apply_along_axis(lambda m: np.convolve(m, k, mode="same"), 1, db)
        L = S.light_field(db)
        L = _resample(L, H, W)[::-1]          # low pitch at the bottom
        L = np.ascontiguousarray(L)
        _cache[key] = L
    L = _cache[key]
    kb = ("B", W, H)
    if kb not in _cache:
        _cache[kb] = S.gauss_blur(L, 0.014 * H)
    return L, _cache[kb]


def base_image(L):
    return S.palette(L)


if __name__ == "__main__":
    from png import write_png
    L, Lw = pane_light(960, 540)
    write_png(os.path.join(TMP, "crop_base.png"), S.to_srgb8(S.palette(L)))
    write_png(os.path.join(TMP, "crop_wide.png"), S.to_srgb8(S.palette(Lw)))
    print("ok", L.shape, L.mean(), L.min(), L.max())
