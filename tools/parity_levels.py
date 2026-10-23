"""Compare levels of the Rust reader with a reference export (project chains only).

Usage: python -I parity_levels.py <rvt.exe> <reference.json> <file.rvt> [...pairs]
"""
import json
import os
import subprocess
import sys
import tempfile

FIELDS = ("id", "name", "elevation", "origin", "uv_box", "type")


def compare(exe, ref_path, rvt):
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "rust.json")
        r = subprocess.run([exe, "levels", rvt, "-o", out], capture_output=True, text=True)
        if r.returncode:
            return 0, [f"RUST FAILED: {r.stderr.strip()[:200]}"]
        rust = {(c["stream"], c["index"]): c["levels"] for c in json.load(open(out, encoding="utf-8"))["chains"]}
    ref = json.load(open(ref_path, encoding="utf-8"))
    n, bad = 0, []
    for p in ref["partitions"]:
        for c in p["chains"]:
            if c.get("virtual_family") is not None or not isinstance(c.get("project"), dict):
                continue
            want = c["project"].get("levels") or []
            got = rust.get((p["stream"], c["index"]))
            if got is None:
                if want:
                    bad.append(f"{p['stream']}#{c['index']}: not a project chain in Rust ({len(want)} levels)")
                continue
            if len(want) != len(got):
                bad.append(f"{p['stream']}#{c['index']}: {len(want)} levels != {len(got)}")
                continue
            for a, b in zip(want, got):
                n += 1
                da = {k: a.get(k) for k in FIELDS}
                db = {k: b.get(k) for k in FIELDS}
                if da != db:
                    bad.append(f"{p['stream']}#{c['index']} level {a['id']}: {da} != {db}")
                    break
    return n, bad


if __name__ == "__main__":
    exe, args = sys.argv[1], sys.argv[2:]
    total = fails = 0
    for i in range(0, len(args), 2):
        n, bad = compare(exe, args[i], args[i + 1])
        total += n
        fails += bool(bad)
        print(os.path.basename(args[i + 1]), "| levels", n, "| mismatches", len(bad), ("| " + "; ".join(bad[:2])) if bad else "", flush=True)
    print("FILES", len(args) // 2, "FAILED", fails, "LEVELS", total)
