# -*- coding: utf-8 -*-
"""Writes the public API's built-in parameter and category names with their English labels to OUTPUT_DIR.

Runs inside Revit through revit_oracle.py --code; opens nothing. Input of gen_builtins.py.
"""
import io
import os

import clr
import System


def esc(s):
    out = []
    for c in s:
        n = ord(c)
        out.append("\\u%04x" % n if n > 126 or n < 32 or c in u'"\\' else c)
    return '"' + "".join(out) + '"'


def row(r):
    return "[%d,%s,%s]" % (r[0], esc(r[1]), esc(r[2]) if r[2] else "null")


def table(enum, label):
    out = []
    for name in System.Enum.GetNames(enum):
        value = System.Enum.Parse(enum, name)
        try:
            text = label(value)
        except Exception:
            text = None
        out.append([int(value), str(name), unicode(text) if text else None])
    return out


data = {
    "parameters": table(clr.GetClrType(DB.BuiltInParameter), DB.LabelUtils.GetLabelFor),
    "categories": table(clr.GetClrType(DB.BuiltInCategory), DB.LabelUtils.GetLabelFor),
}
if not os.path.isdir(OUTPUT_DIR):
    os.makedirs(OUTPUT_DIR)
with io.open(os.path.join(OUTPUT_DIR, "builtins.json"), "w", encoding="utf-8") as f:
    f.write(u'{"parameters":[%s],"categories":[%s]}' % (u",".join(row(r) for r in data["parameters"]), u",".join(row(r) for r in data["categories"])))
print(len(data["parameters"]), len(data["categories"]))
