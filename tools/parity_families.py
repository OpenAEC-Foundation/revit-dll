"""Compare `rvt families` (family_chains, symbol_family) with the reference export catalog.

Usage: python -I parity_families.py <rvt.exe> <reference.json> <file.rvt> [...pairs]
Virtual family chains of Revit 2014 (index >= 1000, step 2168) are not ported and are skipped.
"""
import json
import subprocess
import sys

exe, args = sys.argv[1], sys.argv[2:]
fails = 0
for ref_path, rvt in zip(args[::2], args[1::2]):
    r = subprocess.run([exe, "families", rvt], capture_output=True, text=True, encoding="utf-8")
    if r.returncode:
        print(f"{rvt}: RUST FAILED {r.stderr[:200]}"); fails += 1; continue
    got = json.loads(r.stdout)
    cat = json.load(open(ref_path, encoding="utf-8"))["catalog"]
    want_fc = {k: v for k, v in cat.get("family_chains", {}).items() if v[1] < 1000}
    got_fc = got["family_chains"]
    want_sf, got_sf = cat.get("symbol_family", {}), got["symbol_family"]
    bad = [f"fc {k}: {want_fc.get(k)} != {got_fc.get(k)}" for k in sorted(set(want_fc) | set(got_fc)) if want_fc.get(k) != got_fc.get(k)]
    bad += [f"sf {k}: {want_sf.get(k)} != {got_sf.get(k)}" for k in sorted(set(want_sf) | set(got_sf)) if want_sf.get(k) != got_sf.get(k)]
    fails += bool(bad)
    print(f"{rvt.split('/')[-1]} | family_chains {len(want_fc)} | symbol_family {len(want_sf)} | mismatches {len(bad)} {bad[:3]}")
print(f"FILES {len(args) // 2} FAILED {fails}")
