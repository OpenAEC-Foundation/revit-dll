"""Compare one per-project-chain list of the Rust reader with a reference export.

Usage: python -I parity_lists.py <rvt.exe> <list> <reference.json> <file.rvt> [...pairs]
<list> is both the CLI command and the key in `project` (levels, walls, ...). Records are
compared key by key; floats must match exactly unless RVT_PARITY_TOL is set.
"""
import json
import math
import os
import subprocess
import sys
import tempfile

TOL = float(os.environ.get("RVT_PARITY_TOL", "0"))


def same(a, b):
    if isinstance(a, float) or isinstance(b, float):
        if a is None or b is None:
            return a is b
        return a == b or (TOL > 0 and math.isclose(a, b, rel_tol=TOL, abs_tol=TOL))
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(same(a[k], b[k]) for k in a)
    return a == b


def compare(exe, name, ref_path, rvt):
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "rust.json")
        r = subprocess.run([exe, name, rvt, "-o", out], capture_output=True, text=True)
        if r.returncode:
            return 0, [f"RUST FAILED: {r.stderr.strip()[:200]}"]
        rust = {(c["stream"], c["index"]): c[name] for c in json.load(open(out, encoding="utf-8"))["chains"]}
    ref = json.load(open(ref_path, encoding="utf-8"))
    n, bad = 0, []
    for p in ref["partitions"]:
        for c in p["chains"]:
            if c.get("virtual_family") is not None or not isinstance(c.get("project"), dict):
                continue
            want = c["project"].get(name) or []
            got = rust.get((p["stream"], c["index"]))
            if got is None:
                if want:
                    bad.append(f"{p['stream']}#{c['index']}: not a project chain in Rust ({len(want)} records)")
                continue
            if len(want) != len(got):
                bad.append(f"{p['stream']}#{c['index']}: {len(want)} records != {len(got)}")
                continue
            for a, b in zip(want, got):
                n += 1
                if not same(a, b):
                    diff = {k: (a.get(k), b.get(k)) for k in set(a) | set(b) if not same(a.get(k), b.get(k))}
                    bad.append(f"{p['stream']}#{c['index']} id {a.get('id')}: {diff}")
                    break
    return n, bad


if __name__ == "__main__":
    exe, name, args = sys.argv[1], sys.argv[2], sys.argv[3:]
    total = fails = 0
    for i in range(0, len(args), 2):
        n, bad = compare(exe, name, args[i], args[i + 1])
        total += n
        fails += bool(bad)
        print(os.path.basename(args[i + 1]), "|", name, n, "| mismatches", len(bad), ("| " + "; ".join(bad[:2])[:600]) if bad else "", flush=True)
    print("FILES", len(args) // 2, "FAILED", fails, name.upper(), total)
