"""Minimal PNG writer + 5x7 bitmap font + contact-sheet assembly. No PIL.

Everything a labelled contact sheet needs and nothing else: write_png for a
HxWx3 uint8 array, draw_text/label_cell to burn an ID and its parameters into a
cell, and sheet() to montage equal-sized cells into one titled grid.
"""
import zlib, struct
import numpy as np

def write_png(path, rgb):
    """rgb: HxWx3 uint8"""
    rgb = np.ascontiguousarray(rgb.astype(np.uint8))
    h, w, _ = rgb.shape
    rows = np.concatenate(
        [np.zeros((h, 1), np.uint8), rgb.reshape(h, w * 3)], axis=1
    )
    raw = rows.tobytes()
    comp = zlib.compress(raw, 6)

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)
    out = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", comp) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(out)
    return path


def read_png_size(path):
    with open(path, "rb") as f:
        d = f.read(33)
    return struct.unpack(">II", d[16:24])


# ---------------------------------------------------------------- 5x7 font
_F = {
 'A': "01110/10001/10001/11111/10001/10001/10001",
 'B': "11110/10001/10001/11110/10001/10001/11110",
 'C': "01110/10001/10000/10000/10000/10001/01110",
 'D': "11110/10001/10001/10001/10001/10001/11110",
 'E': "11111/10000/10000/11110/10000/10000/11111",
 'F': "11111/10000/10000/11110/10000/10000/10000",
 'G': "01110/10001/10000/10111/10001/10001/01111",
 'H': "10001/10001/10001/11111/10001/10001/10001",
 'I': "11111/00100/00100/00100/00100/00100/11111",
 'J': "00111/00010/00010/00010/00010/10010/01100",
 'K': "10001/10010/10100/11000/10100/10010/10001",
 'L': "10000/10000/10000/10000/10000/10000/11111",
 'M': "10001/11011/10101/10101/10001/10001/10001",
 'N': "10001/11001/10101/10011/10001/10001/10001",
 'O': "01110/10001/10001/10001/10001/10001/01110",
 'P': "11110/10001/10001/11110/10000/10000/10000",
 'Q': "01110/10001/10001/10001/10101/10010/01101",
 'R': "11110/10001/10001/11110/10100/10010/10001",
 'S': "01111/10000/10000/01110/00001/00001/11110",
 'T': "11111/00100/00100/00100/00100/00100/00100",
 'U': "10001/10001/10001/10001/10001/10001/01110",
 'V': "10001/10001/10001/10001/10001/01010/00100",
 'W': "10001/10001/10001/10101/10101/11011/10001",
 'X': "10001/10001/01010/00100/01010/10001/10001",
 'Y': "10001/10001/01010/00100/00100/00100/00100",
 'Z': "11111/00001/00010/00100/01000/10000/11111",
 '0': "01110/10001/10011/10101/11001/10001/01110",
 '1': "00100/01100/00100/00100/00100/00100/01110",
 '2': "01110/10001/00001/00010/00100/01000/11111",
 '3': "11111/00010/00100/00010/00001/10001/01110",
 '4': "00010/00110/01010/10010/11111/00010/00010",
 '5': "11111/10000/11110/00001/00001/10001/01110",
 '6': "00110/01000/10000/11110/10001/10001/01110",
 '7': "11111/00001/00010/00100/01000/01000/01000",
 '8': "01110/10001/10001/01110/10001/10001/01110",
 '9': "01110/10001/10001/01111/00001/00010/01100",
 ' ': "00000/00000/00000/00000/00000/00000/00000",
 '.': "00000/00000/00000/00000/00000/01100/01100",
 ',': "00000/00000/00000/00000/01100/01100/01000",
 '-': "00000/00000/00000/11111/00000/00000/00000",
 '+': "00000/00100/00100/11111/00100/00100/00000",
 '/': "00001/00010/00010/00100/01000/01000/10000",
 ':': "00000/01100/01100/00000/01100/01100/00000",
 '=': "00000/00000/11111/00000/11111/00000/00000",
 '(': "00010/00100/01000/01000/01000/00100/00010",
 ')': "01000/00100/00010/00010/00010/00100/01000",
 '%': "11001/11010/00010/00100/01000/01011/10011",
 '_': "00000/00000/00000/00000/00000/00000/11111",
 '*': "00000/10101/01110/11111/01110/10101/00000",
 '<': "00010/00100/01000/10000/01000/00100/00010",
 '>': "01000/00100/00010/00001/00010/00100/01000",
 '#': "01010/11111/01010/01010/01010/11111/01010",
 '!': "00100/00100/00100/00100/00100/00000/00100",
 '?': "01110/10001/00001/00010/00100/00000/00100",
 "'": "00100/00100/00000/00000/00000/00000/00000",
}
_GLYPH = {k: np.array([[c == '1' for c in row] for row in v.split('/')], bool)
          for k, v in _F.items()}


def text_width(s, scale=2, sp=1):
    return len(s) * (5 + sp) * scale


def draw_text(img, x, y, s, scale=2, color=(255, 255, 255), bg=None, sp=1):
    """img HxWx3 uint8, in place."""
    s = s.upper()
    H, W, _ = img.shape
    w = text_width(s, scale, sp)
    h = 7 * scale
    if bg is not None:
        x0, y0 = max(0, x - scale), max(0, y - scale)
        x1, y1 = min(W, x + w + scale), min(H, y + h + scale)
        if x1 > x0 and y1 > y0:
            img[y0:y1, x0:x1] = (
                img[y0:y1, x0:x1].astype(np.int32) * 0 + np.array(bg, np.int32)
            ).astype(np.uint8) if len(bg) == 3 else img[y0:y1, x0:x1]
    col = np.array(color, np.uint8)
    cx = x
    for ch in s:
        g = _GLYPH.get(ch, _GLYPH['?'])
        gg = np.repeat(np.repeat(g, scale, 0), scale, 1)
        gh, gw = gg.shape
        x0, y0 = cx, y
        x1, y1 = min(W, x0 + gw), min(H, y0 + gh)
        if x0 < W and y0 < H and x1 > 0 and y1 > 0:
            sx0, sy0 = max(0, -x0), max(0, -y0)
            sub = gg[sy0:sy0 + (y1 - max(0, y0)), sx0:sx0 + (x1 - max(0, x0))]
            tgt = img[max(0, y0):y1, max(0, x0):x1]
            tgt[sub] = col
        cx += (5 + sp) * scale
    return img


def label_cell(cell, lines, scale=2, pad=4):
    """Draw label lines top-left on a dark strip."""
    h = len(lines) * (7 * scale + 3) + 2 * pad
    w = max(text_width(l, scale) for l in lines) + 2 * pad
    w = min(w, cell.shape[1])
    h = min(h, cell.shape[0])
    strip = cell[:h, :w].astype(np.float32) * 0.25
    cell[:h, :w] = strip.astype(np.uint8)
    y = pad
    for l in lines:
        draw_text(cell, pad, y, l, scale, (255, 255, 255))
        y += 7 * scale + 3
    return cell


def sheet(cells, cols, gap=6, bgcol=(24, 24, 28), title=None, tscale=3):
    """cells: list of HxWx3 uint8, all same size."""
    n = len(cells)
    rows = (n + cols - 1) // cols
    ch, cw, _ = cells[0].shape
    top = 0 if title is None else (7 * tscale + 14)
    H = top + rows * ch + (rows + 1) * gap
    W = cols * cw + (cols + 1) * gap
    out = np.zeros((H, W, 3), np.uint8)
    out[:] = np.array(bgcol, np.uint8)
    if title is not None:
        draw_text(out, gap, 6, title, tscale, (235, 235, 240))
    for i, c in enumerate(cells):
        r, k = divmod(i, cols)
        y = top + gap + r * (ch + gap)
        x = gap + k * (cw + gap)
        out[y:y + ch, x:x + cw] = c
    return out
