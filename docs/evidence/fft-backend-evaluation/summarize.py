"""Print paired timing and resource summaries from retained CSV files."""
import csv
import pathlib
import statistics as st
import sys

root = pathlib.Path(sys.argv[1])
rows = {}
for pair in range(7):
    for backend in ["baseline", "candidate"]:
        with (root / f"pair-{pair}-{backend}.csv").open() as f:
            for r in csv.DictReader(f):
                rows[pair, backend, int(r["n"]), int(r["tapers"]), r["input"]] = r
print("| N | Tapers | Baseline µs | RealFFT µs | Paired saved µs (median; min–max) | Saved % |")
print("|---:|---:|---:|---:|---:|---:|")
for n in [4096, 8192, 16384]:
    for k in [1, 3, 5]:
        def values(backend, field, kind="mixed"):
            return [float(rows[p, backend, n, k, kind][field]) for p in range(7)]
        a, b = [values(backend, "ns_per_column") for backend in ["baseline", "candidate"]]
        d = [(x-y)/1000 for x, y in zip(a, b)]
        ratio = [(x-y)/x*100 for x, y in zip(a, b)]
        print(f"| {n} | {k} | {st.median(a)/1000:.2f} | {st.median(b)/1000:.2f} | "
            f"{st.median(d):.2f}; {min(d):.2f}–{max(d):.2f} | {st.median(ratio):.1f} |")
print("\n| N/tapers | Retained bytes B/C | Construction peak B/C | Init µs B/C | Taper reconfigure µs B/C | Silence µs B/C |")
print("|---|---:|---:|---:|---:|---:|")
for n, k in [(4096,1), (8192,1), (16384,1), (8192,5), (16384,5)]:
    parts = []
    for field, divisor, kind in [("retained_bytes",1,"mixed"), ("construct_peak_bytes",1,"mixed"),
        ("init_ns",1000,"mixed"), ("reconfigure_ns",1000,"mixed"), ("ns_per_column",1000,"silence")]:
        parts.append(" / ".join(f"{st.median(float(rows[p,b,n,k,kind][field]) for p in range(7))/divisor:.2f}"
            for b in ["baseline", "candidate"]))
    print(f"| {n}/{k} | " + " | ".join(parts) + " |")
assert all(r["steady_allocs"] == "0" for r in rows.values())
