"""Print an `rvt object` / `rvt objects` JSON compactly: one line per field, nested objects
indented, pointer targets listed after the root (par C, for reading object views).

Usage: rvt objects <file> --class Level --limit 1 | python -I object_brief.py [max_depth]
"""
import json
import sys

DEPTH = int(sys.argv[1]) if len(sys.argv) > 1 else 3


def short(v, d):
    if isinstance(v, dict) and "fields" in v:
        if d >= DEPTH:
            return v["class"] + "{...}"
        return v["class"] + "{" + ", ".join(f"{f['name']}={short(f['value'], d + 1)}" for f in v["fields"]) + "}"
    if isinstance(v, list):
        items = [short(x, d) for x in v[:6]]
        return "[" + ", ".join(items) + (f", ...{len(v)}" if len(v) > 6 else "") + "]"
    if isinstance(v, float):
        return f"{v:.6g}"
    return json.dumps(v, ensure_ascii=False)


def show(o):
    print(f"#{o['id']} {o['class']} <- {o['parents']} cat={o.get('category')} complete={o['complete']}")
    for f in o["fields"]:
        print(f"  {f['name']}: {f['type']} = {short(f['value'], 1)}")
    for i, t in enumerate(o.get("objects", [])):
        print(f"  [{i}] {short(t, 1)}")


d = json.load(sys.stdin)
for o in ([d] if "id" in d else d.get("objects", [])):
    show(o)
