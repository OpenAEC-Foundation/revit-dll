"""Run `rvt check` over the Revit files under a folder and report per file what decodes.

Usage: python -I scan_files.py <rvt.exe> <folder> <report.jsonl> [--max-mb N] [--per-format N]
Reads files only. Files are taken smallest first, and at most N per extension and format, so one
run shows the spread of versions before it spends time on many similar files. The report names
model files: keep it out of git (`exports/`).
"""
import argparse
import json
import os
import re
import subprocess
import time

EXT = {".rvt", ".rfa", ".rte", ".rft"}
BACKUP = re.compile(r"\.\d{4}\.(rvt|rfa|rte|rft)$", re.I)


def basic_format(path):
    """The `Format: 20xx` line of BasicFileInfo, without parsing the compound file."""
    with open(path, "rb") as f:
        data = f.read(4 * 1024 * 1024)
    needle = "Format: ".encode("utf-16le")
    i = data.find(needle)
    if i < 0:
        return None
    return data[i + len(needle) : i + len(needle) + 8].decode("utf-16le", "replace")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("exe")
    ap.add_argument("folder")
    ap.add_argument("report")
    ap.add_argument("--max-mb", type=int, default=120)
    ap.add_argument("--per-format", type=int, default=3)
    ap.add_argument("--timeout", type=int, default=600)
    a = ap.parse_args()

    files = []
    for root, dirs, names in os.walk(a.folder):
        dirs[:] = [d for d in dirs if not d.startswith((".", "#")) and not d.endswith("_backup")]
        for n in names:
            if os.path.splitext(n)[1].lower() in EXT and not BACKUP.search(n):
                p = os.path.join(root, n)
                try:
                    size = os.path.getsize(p)
                except OSError:
                    continue
                if size <= a.max_mb * 1024 * 1024:
                    files.append((size, p))
    files.sort()
    print(len(files), "files", flush=True)

    seen = {}
    with open(a.report, "a", encoding="utf-8") as out:
        for size, p in files:
            try:
                fmt = basic_format(p)
            except OSError as e:
                print("unreadable", p, e, flush=True)
                continue
            key = (os.path.splitext(p)[1].lower(), fmt)
            if seen.get(key, 0) >= a.per_format:
                continue
            seen[key] = seen.get(key, 0) + 1
            t = time.time()
            row = {"path": p, "mb": round(size / 1048576, 1), "ext": key[0], "format": fmt}
            try:
                r = subprocess.run([a.exe, "check", p], capture_output=True, text=True, encoding="utf-8", timeout=a.timeout)
                if r.returncode:
                    row["error"] = r.stderr.strip()[:300]
                else:
                    c = json.loads(r.stdout)
                    row.update(decoded=c["decoded"], failed=c["failed"], by_class=c["by_class"])
            except (subprocess.TimeoutExpired, ValueError) as e:
                row["error"] = str(e)[:200]
            row["seconds"] = round(time.time() - t, 1)
            out.write(json.dumps(row, ensure_ascii=False) + "\n")
            out.flush()
            print(row["ext"], row["format"], row["mb"], "decoded", row.get("decoded"), "failed", row.get("failed"), (row.get("error") or "")[:100], os.path.basename(p), flush=True)


if __name__ == "__main__":
    main()
