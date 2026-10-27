"""Compare two directories of `graph_digest` outputs (par C): records that decoded before must
decode to the same digest after a decoder change.

Usage: python -I digest_compare.py <old_dir> <new_dir>
"""
import collections
import os
import sys


def load(path):
    out = {}
    for line in open(path, encoding="utf-8"):
        ch, i, ok, h = line.rstrip("\n").split("\t")
        out[(ch, i)] = (ok, h)
    return out


def main(old, new):
    tot = collections.Counter()
    per_channel = collections.Counter()
    for fn in sorted(os.listdir(old)):
        if not fn.endswith(".tsv") or not os.path.exists(os.path.join(new, fn)):
            continue
        a, b = load(os.path.join(old, fn)), load(os.path.join(new, fn))
        for k, (ok, h) in a.items():
            n = b.get(k)
            if ok == "ok":
                if n is None:
                    tot["ok_missing"] += 1
                elif n[0] != "ok":
                    tot["ok_now_err"] += 1
                    print("REGRESS", fn, k)
                elif n[1] != h:
                    tot["ok_changed"] += 1
                    print("CHANGED", fn, k)
                else:
                    tot["ok_same"] += 1
            elif n and n[0] == "ok":
                tot["err_now_ok"] += 1
                per_channel[k[0]] += 1
            else:
                tot["err_still"] += 1
    print(dict(tot), "fixed per channel:", dict(per_channel))
    return 1 if tot["ok_now_err"] or tot["ok_changed"] or tot["ok_missing"] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
