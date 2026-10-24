"""Compare a family-chain list by record id (for lists ported in parts): records Rust produces must
equal the reference on RVT_PARITY_KEYS; reference-only records are counted, Rust-only are errors.

Usage: python -I parity_byid.py <rvt.exe> <list> <reference.json> <file.rvt> [...pairs]
"""
import json, os, subprocess, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from parity_lists import same  # noqa: E402

exe, name, args = sys.argv[1], sys.argv[2], sys.argv[3:]
KEYS = [k for k in os.environ.get("RVT_PARITY_KEYS", "").split(",") if k]
tot = bad = ref_only = rust_only = 0
for ref_path, rvt in zip(args[::2], args[1::2]):
    r = subprocess.run([exe, name, rvt], capture_output=True, text=True, encoding="utf-8")
    if r.returncode:
        print(rvt, "RUST FAILED", r.stderr[:200]); bad += 1; continue
    got = {(c["stream"], c["index"]): {x["id"]: x for x in c[name]} for c in json.loads(r.stdout)["chains"]}
    D = json.load(open(ref_path, encoding="utf-8")); fb = 0; msg = []
    for p in D["partitions"]:
        for c in p["chains"]:
            if c["index"] >= 1000 or isinstance(c.get("project"), dict) or isinstance(c.get("document"), dict):
                continue
            want = {x["id"]: x for x in c.get(name) or []}; g = got.get((p["stream"], c["index"]), {})
            for i, a in want.items():
                if i not in g:
                    ref_only += 1; continue
                tot += 1; b = g[i]
                aa = {k: a.get(k) for k in KEYS}; bb = {k: b.get(k) for k in KEYS}
                if not same(aa, bb):
                    fb += 1; msg.append(f"id {i}: " + str({k: (aa[k], bb[k]) for k in KEYS if not same(aa[k], bb[k])})[:300])
            rust_only += len(set(g) - set(want))
    bad += fb
    if fb: print(rvt.split("/")[-1], "mismatches", fb, msg[:2])
print(f"compared {tot} | mismatches {bad} | reference only {ref_only} | rust only {rust_only}")
