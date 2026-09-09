"""Compare regenerated native-f32 dumps; no third-party Python packages."""
import array
import csv
import math
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
def floats(path):
    a = array.array("f")
    a.frombytes(path.read_bytes())
    if sys.byteorder != "little":
        a.byteswap()
    return a

a, b = [floats(root / f"{name}.bin") for name in ["baseline", "candidate"]]
sa, sb = [floats(root / f"{name}.bin.smooth") for name in ["baseline", "candidate"]]
qa, qb = [(root / f"{name}.bin.db").read_bytes() for name in ["baseline", "candidate"]]
assert len(a) == len(b) == len(sa) == len(sb) == len(qa) == len(qb)
w = csv.writer(sys.stdout, lineterminator="\n")
w.writerow(["n", "tapers", "input", "buckets", "unequal", "max_abs_power",
    "max_db_above_minus120", "rms_db_above_minus120", "max_relative_above_minus120", "quantized_unequal",
    "max_byte_delta", "max_smoothed_abs_power", "peak_index_changed_columns"])
offset = 0
bins = 3828
for n in [4096, 8192, 16384]:
    for k in [1, 3, 5]:
        for kind in ["mixed", "silence", "tone", "antiphase", "quiet", "noise", "impulse", "golden"]:
            cols = 192 if kind == "golden" else 16
            count = cols * bins
            end = offset + count
            unequal = qunequal = dcount = peak_changes = 0
            max_abs = max_db = sum_db2 = max_byte = max_smooth = max_relative = 0.0
            for i in range(offset, end):
                x, y = a[i], b[i]
                unequal += x != y
                max_abs = max(max_abs, abs(x-y))
                if max(x, y) > 1e-12:
                    max_relative = max(max_relative, abs(x-y) / max(x, y))
                    db = abs(10 * math.log10(max(x, 1e-12) / max(y, 1e-12)))
                    max_db = max(max_db, db)
                    sum_db2 += db*db
                    dcount += 1
                qunequal += qa[i] != qb[i]
                max_byte = max(max_byte, abs(qa[i] - qb[i]))
                max_smooth = max(max_smooth, abs(sa[i]-sb[i]))
            for start in range(offset, end, bins):
                ia = max(range(bins), key=lambda j: a[start+j])
                ib = max(range(bins), key=lambda j: b[start+j])
                peak_changes += ia != ib
            w.writerow([n, k, kind, count, unequal, f"{max_abs:.9e}", f"{max_db:.9e}",
                f"{math.sqrt(sum_db2 / max(dcount, 1)):.9e}", f"{max_relative:.9e}", qunequal, int(max_byte),
                f"{max_smooth:.9e}", peak_changes])
            offset = end
assert offset == len(a), (offset, len(a))
