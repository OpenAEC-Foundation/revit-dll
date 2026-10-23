"""Compare element tables of the Rust reader with a reference export of the clean-room reader.

Usage: python -I parity_elements.py <rvt.exe> <reference.json> <file.rvt> [...pairs]
Prints one line per file: partitions, chains, elements compared and mismatches.
"""
import json
import subprocess
import sys
import tempfile
import os


def key(c):
    return (c["index"],)


def compare(exe, ref_path, rvt):
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "rust.json")
        r = subprocess.run([exe, "elements", rvt, "-o", out], capture_output=True, text=True)
        if r.returncode:
            return f"RUST FAILED: {r.stderr.strip()[:200]}"
        rust = json.load(open(out, encoding="utf-8"))
    ref = json.load(open(ref_path, encoding="utf-8"))
    rp = {p["stream"]: p for p in rust["partitions"]}
    n_el = n_ch = 0
    bad = []
    for p in ref["partitions"]:
        q = rp.get(p["stream"])
        if q is None:
            bad.append(f"missing partition {p['stream']}")
            continue
        qc = {c["index"]: c for c in q["chains"]}
        for c in p["chains"]:
            if c.get("virtual_family") is not None:   # step 2168: derived chains, not in the file
                continue
            d = qc.get(c["index"])
            if d is None:
                bad.append(f"{p['stream']}#{c['index']} missing")
                continue
            n_ch += 1
            if (c["version"], c["bits"]) != (d["version"], d["bits"]):
                bad.append(f"{p['stream']}#{c['index']} version/bits {c['version'], c['bits']} != {d['version'], d['bits']}")
            if len(c["elements"]) != len(d["elements"]):
                bad.append(f"{p['stream']}#{c['index']} elements {len(c['elements'])} != {len(d['elements'])}")
                continue
            for a, b in zip(c["elements"], d["elements"]):
                n_el += 1
                if (a["id"], a["cat"], a["tag"], a["bbox"]) != (b["id"], b["cat"], b["tag"], b["bbox"]):
                    bad.append(f"{p['stream']}#{c['index']} element {a['id']}: {a} != {b}")
                    break
    return f"chains {n_ch}, elements {n_el}, mismatches {len(bad)}" + ("" if not bad else " | " + "; ".join(bad[:3]))


if __name__ == "__main__":
    exe = sys.argv[1]
    args = sys.argv[2:]
    fails = 0
    for i in range(0, len(args), 2):
        res = compare(exe, args[i], args[i + 1])
        fails += "mismatches 0" not in res
        print(os.path.basename(args[i + 1]), "|", res, flush=True)
    print("FILES", len(args) // 2, "FAILED", fails)
