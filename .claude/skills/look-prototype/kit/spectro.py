"""Real-recording log-frequency STFT -> a 0..1 'light' field, plus the palette.

Decodes an excerpt of a take through ffmpeg (cached beside this file), folds it
to a log-frequency dB picture, and normalises that to the 0..1 level a pane
would draw. The palette here is the prototype's, not the plugin's.
"""
import os, subprocess, sys, zlib
import numpy as np

TMP = os.path.dirname(os.path.abspath(__file__))
TAKES = os.path.expanduser(os.environ.get("HARMONIGRAPH_TAKES", "~/Music/Harmonigraph Takes"))
# The take every script here defaults to. HARMONIGRAPH_TAKE names another: a
# file inside TAKES, or an absolute path (os.path.join keeps that one whole).
DEFAULT_TAKE = os.path.join(TAKES, os.environ.get("HARMONIGRAPH_TAKE", "take-2026-09-11_03-16-17.wav"))


def missing_take(wav):
    return SystemExit(
        "take not found: %s\n"
        "Point at another take with HARMONIGRAPH_TAKES=<directory> (default "
        "~/Music/Harmonigraph Takes) and/or HARMONIGRAPH_TAKE=<file name inside "
        "it, or an absolute path>. HARMONIGRAPH_TAKE_START / _DUR move the "
        "excerpt window (default 72 s for 20 s)." % wav
    )


def cache_name(prefix, wav, start, dur, ext):
    """Cache file name; changes when the take, its path, or the window does."""
    tag = zlib.crc32(os.path.abspath(wav).encode("utf-8")) & 0xFFFFFFFF
    return "%s_%s_%08x_%g_%g%s" % (prefix, os.path.basename(wav)[:20], tag, start, dur, ext)


def decode(wav, start, dur, sr=22050):
    raw = os.path.join(TMP, cache_name("cache", wav, start, dur, ".raw"))
    if not os.path.exists(raw):
        if not os.path.exists(wav):
            raise missing_take(wav)
        subprocess.run(
            ["ffmpeg", "-v", "error", "-y", "-ss", str(start), "-t", str(dur),
             "-i", wav, "-ac", "1", "-ar", str(sr), "-f", "s16le", raw],
            check=True,
        )
    x = np.fromfile(raw, dtype=np.int16).astype(np.float32) / 32768.0
    return x, sr


def stft_log(x, sr, n_fft=2048, hop=128, fmin=70.0, fmax=9000.0, rows=512):
    win = np.hanning(n_fft).astype(np.float32)
    n = 1 + (len(x) - n_fft) // hop
    idx = np.arange(n_fft)[None, :] + hop * np.arange(n)[:, None]
    frames = x[idx] * win
    spec = np.abs(np.fft.rfft(frames, axis=1))  # n x (n_fft/2+1)
    freqs = np.fft.rfftfreq(n_fft, 1.0 / sr)
    # log-frequency rows, each row = max of the bins that fall in its band
    edges = np.exp(np.linspace(np.log(fmin), np.log(fmax), rows + 1))
    out = np.zeros((rows, n), np.float32)
    for r in range(rows):
        lo, hi = edges[r], edges[r + 1]
        sel = (freqs >= lo) & (freqs < hi)
        if not sel.any():
            # below bin spacing: interpolate
            f = 0.5 * (lo + hi)
            k = np.clip(np.searchsorted(freqs, f), 1, len(freqs) - 1)
            t = (f - freqs[k - 1]) / (freqs[k] - freqs[k - 1])
            out[r] = spec[:, k - 1] * (1 - t) + spec[:, k] * t
        else:
            out[r] = spec[:, sel].max(axis=1)
    db = 20.0 * np.log10(out + 1e-6)
    return db


def light_field(db, floor_db=None, top_db=None):
    hi = np.percentile(db, 99.7) if top_db is None else top_db
    lo = hi - 66.0 if floor_db is None else floor_db
    v = (db - lo) / (hi - lo)
    return np.clip(v, 0.0, 1.0).astype(np.float32)


# ------------------------------------------------------------------ palette
# near-black -> deep blue/purple -> magenta/orange -> pale yellow
_STOPS = np.array([
    [0.00, 0.020, 0.020, 0.045],
    [0.14, 0.070, 0.045, 0.190],
    [0.32, 0.240, 0.070, 0.430],
    [0.50, 0.540, 0.110, 0.470],
    [0.66, 0.820, 0.250, 0.290],
    [0.80, 0.960, 0.500, 0.150],
    [0.91, 0.995, 0.760, 0.290],
    [1.00, 1.000, 0.960, 0.760],
], np.float32)


def palette(t):
    t = np.clip(t, 0.0, 1.0)
    p = _STOPS[:, 0]
    out = np.empty(t.shape + (3,), np.float32)
    for c in range(3):
        out[..., c] = np.interp(t, p, _STOPS[:, 1 + c])
    return out


def to_srgb8(rgb):
    return np.clip(rgb * 255.0 + 0.5, 0, 255).astype(np.uint8)


def _box(a, r, axis):
    if r < 1:
        return a
    pad = [(0, 0), (0, 0)]
    pad[axis] = (r + 1, r)
    b = np.pad(a, pad, mode="edge")
    c = np.cumsum(b, axis=axis, dtype=np.float32)
    if axis == 0:
        return (c[2 * r + 1:] - c[:-(2 * r + 1)]) / (2 * r + 1)
    return (c[:, 2 * r + 1:] - c[:, :-(2 * r + 1)]) / (2 * r + 1)


def gauss_blur(a, sigma):
    """Three box passes per axis: a fast, accurate-enough Gaussian."""
    if sigma <= 0:
        return a.astype(np.float32).copy()
    r = max(1, int(round(sigma * 0.85)))
    b = a.astype(np.float32)
    for _ in range(3):
        b = _box(_box(b, r, 0), r, 1)
    return b.astype(np.float32)


if __name__ == "__main__":
    from png import write_png, draw_text
    wav = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_TAKE
    x, sr = decode(wav, 0, 95)
    db = stft_log(x, sr, hop=512, rows=256)
    L = light_field(db)
    img = to_srgb8(palette(L[::-1]))
    # downsample time to ~1400 wide
    step = max(1, img.shape[1] // 1400)
    img = img[:, ::step]
    print("preview", img.shape, os.path.basename(wav))
    write_png(os.path.join(TMP, "preview_%s.png" % os.path.basename(wav)[:-4]), img)
