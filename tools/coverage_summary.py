"""Summarise `rvt coverage` outputs (par C): records and completely decoded records per Revit
version and channel, and the classes that fail most over all files.

Usage: python -I coverage_summary.py <dir with coverage *.json> [--files]
"""
import collections
import json
import os
import sys


def main(d, per_file=False):
    tot = collections.defaultdict(lambda: [0, 0, 0, 0])  # (version, channel) -> files, records, empty, complete
    fails = collections.defaultdict(lambda: [0, 0, "", "", 0])  # (channel, class) -> failed, records, reason, example, files
    no_records = []
    for fn in sorted(os.listdir(d)):
        if not fn.endswith(".json"):
            continue
        c = json.load(open(os.path.join(d, fn), encoding="utf-8"))
        v = c.get("version")
        if not c["channels"]:
            no_records.append((fn, v))
        for ch in c["channels"]:
            t = tot[(v, ch["channel"])]
            t[0] += 1
            t[1] += ch["records"]
            t[2] += ch["empty"]
            t[3] += ch["complete"]
            for f in ch["failing"]:
                e = fails[(ch["channel"], f["class"])]
                e[0] += f["failed"]
                e[1] += f["records"]
                if not e[2]:
                    e[2], e[3] = f["reason"], f"{fn[:-5]} {f['example']}"
                e[4] += 1
            if per_file and ch["complete"] + ch["empty"] < ch["records"]:
                print(f"  {fn[:-5]} ch{ch['channel']}: {ch['complete']}/{ch['records'] - ch['empty']}")
    print("version channel files records empty complete pct")
    for (v, ch), (n, r, e, ok) in sorted(tot.items(), key=lambda x: (x[0][0] or 0, x[0][1])):
        print(f"{v} {ch} {n} {r} {e} {ok} {100.0 * ok / max(1, r - e):.3f}%")
    print("top failing classes (channel, class, failed/records, files, reason, example):")
    for (ch, cl), (f, r, reason, ex, nf) in sorted(fails.items(), key=lambda x: -x[1][0])[:15]:
        print(f"  {ch} {cl} {f}/{r} files={nf} | {reason} | {ex}")
    if no_records:
        print("files without object records:", no_records)


if __name__ == "__main__":
    main(sys.argv[1], "--files" in sys.argv)
