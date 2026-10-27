"""Sum `object_parity` JSON lines (par C): per list and field the matches over all files, the
files with a mismatch and some examples.

Usage: python -I parity_objects_summary.py <file with one JSON line per sample>
"""
import collections
import json
import sys

tot = collections.defaultdict(lambda: [0, 0])
recs = collections.Counter()
missing = collections.Counter()
bad_files = collections.defaultdict(list)
examples = collections.defaultdict(list)
files = 0
for line in open(sys.argv[1], encoding="utf-8"):
    line = line.strip()
    if not line.startswith("{"):
        continue
    d = json.loads(line)
    files += 1
    name = d["file"].replace("\\", "/").rsplit("/", 1)[-1]
    for lst in ("levels", "walls", "materials"):
        t = d[lst]
        recs[lst] += t["records"]
        missing[lst] += t["missing_object"]
        for f, mn in t["fields"].items():
            m, n = map(int, mn.split("/"))
            tot[(lst, f)][0] += m
            tot[(lst, f)][1] += n
        if not t["all_fields_equal"]:
            bad_files[lst].append(name)
            examples[lst].extend(t["examples"][:2])
print(f"files: {files}")
for lst in ("levels", "walls", "materials"):
    print(f"{lst}: {recs[lst]} records, {missing[lst]} without object")
    for (l, f), (m, n) in sorted(tot.items()):
        if l == lst:
            print(f"  {f}: {m}/{n}")
    if bad_files[lst]:
        print(f"  files with a mismatch: {bad_files[lst]}")
        for e in examples[lst][:8]:
            print(f"    {e}")
